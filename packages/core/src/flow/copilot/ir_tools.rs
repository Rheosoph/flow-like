//! Stateful tool surface for planning, building, validating, and atomically committing typed IR.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

#[cfg(test)]
use std::sync::Condvar;

use flow_like_ast::model::{
    Arg as AstArg, Block as AstBlock, BoardAst, Call as AstCall, EventBlock as AstEventBlock,
    Expr as AstExpr, FnDecl as AstFnDecl, Literal as AstLiteral, Param as AstParam,
    Stmt as AstStmt,
};
use flow_like_types::create_id;
use rig::{completion::ToolDefinition, tool::Tool};
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::ir::{
    FlowCapabilityPlan, FlowCapabilityPlanRequest, FlowIrArg, FlowIrCompileResult, FlowIrContainer,
    FlowIrDataType, FlowIrDiagnostic, FlowIrInterface, FlowIrLiteral, FlowIrModule,
    FlowIrObjectField, FlowIrParam, FlowIrProgram, FlowIrStep, FlowIrType, FlowIrValue,
    FlowIrVariable, FlowModuleKind, MAX_FLOW_IR_CAPABILITY_REQUIREMENTS,
    MAX_FLOW_IR_PIN_REQUIREMENTS_PER_DIRECTION, ReachableFlowIrOccurrence, compile_flow_ir,
    plan_flow_capabilities, reachable_flow_ir_occurrences, validate_flow_capability_usage,
    validate_ir_resource_limits,
};
use super::provider::{CatalogProvider, metadata_to_signature};
use super::tools::{
    FlowScriptCandidateProfile, FlowScriptCandidateRegression, board_has_no_nodes,
    detect_flowscript_candidate_regression, flowscript_workspace_tag, profile_flowscript_candidate,
    render_edit_flowscript_result,
};
use super::types::{BoardCommand, FlowIrCommitToken, NodeMetadata, PinMetadata};
use crate::flow::ast::{
    FlowScriptDiagnostic, FlowScriptDiagnosticCode, FlowScriptDiagnosticFix,
    FlowScriptDiagnosticPhase, ReconcileMode, ReconcileResult, RenderOptions, board_to_flowscript,
    destructive_flowscript_command_summaries, reconcile_with_catalog_mode,
};
use crate::flow::board::Board;

const MAX_FLOW_IR_DRAFTS_PER_STORE: usize = 32;
const MAX_FLOWSCRIPT_DRAFTS_PER_STORE: usize = 32;
const MAX_FLOW_IR_DRAFT_ID_BYTES: usize = 128;
const MAX_FLOWSCRIPT_SOURCE_BYTES: usize = 1_048_576;
const MAX_FLOWSCRIPT_DRAFT_STORE_BYTES: usize = 8 * 1_048_576;
const MAX_FLOWSCRIPT_REPAIR_DECLARATIONS: usize = 3;
const MAX_FLOWSCRIPT_REPAIR_COMPANION_DECLARATIONS: usize = 8;
const MIN_FLOWSCRIPT_REPAIR_DECLARATION_SIMILARITY: f64 = 0.86;
const MAX_FLOW_IR_CAPABILITY_PLAN_BYTES: usize = 262_144;
const MAX_FLOW_IR_DRAFT_STORE_BYTES: usize = 8 * 1_048_576;
const MAX_FLOW_IR_AUTHORED_NAME_BYTES: usize = 256;
const MAX_FLOW_IR_ENTITY_ALLOWLIST_ITEMS: usize = 4_096;
const MAX_FLOW_IR_CAPABILITY_INTENT_BYTES: usize = 4_096;
const MAX_FLOW_IR_PIN_ALIASES_PER_REQUIREMENT: usize = 32;
const MAX_FLOW_IR_ACCEPTANCE_CONTRACTS_PER_STORE: usize = 64;
const MAX_FLOW_IR_ACCEPTANCE_PROMPT_CHARS: usize = 65_536;
const MAX_FLOW_IR_ACCEPTANCE_CRITERIA: usize = 32;
const MAX_FLOW_IR_ACCEPTANCE_SUMMARY_CHARS: usize = 240;
const FLOW_IR_REQUEST_IDENTITY_DOMAIN: &[u8] = b"flowpilot.request-identity/v1\0";
const FLOW_IR_UNBOUND_IDENTITY_DOMAIN: &[u8] = b"flowpilot.request-identity/unbound/v1\0";
const FLOWSCRIPT_CATALOG_FINGERPRINT_DOMAIN: &[u8] =
    b"flowpilot.flowscript-catalog-fingerprint/v1\0";

/// A deliberately small, host-derived guardrail for explicit multi-part requests. The model can
/// choose catalog declarations, module boundaries, and implementation details, but it cannot make
/// one of these independently stated requirements disappear from its own required capability plan.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct RequestAcceptanceCriterion {
    summary: String,
    actions: Vec<String>,
    /// Host-selected protocol/service subjects. Generic payload nouns (message, notification,
    /// record, and so on) are intentionally excluded so email cannot satisfy Slack by sharing
    /// only the word "notification".
    objects: Vec<String>,
    forbidden: bool,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
struct RequestAcceptanceContract {
    criteria: Vec<RequestAcceptanceCriterion>,
    /// Host-derived prohibitions the machine could not enforce: recipient/timing-scoped bans that
    /// need dataflow proof, and bans dropped because they contradicted an explicit requirement.
    /// They never block or weaken the remaining criteria; they are surfaced once in the check and
    /// commit messages so the human review sees exactly which bans were left to it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    omitted_prohibitions: Vec<String>,
    /// A structural host predicate for human-in-the-loop approval requests. Unlike the generic
    /// catalog criteria above, this proves branch placement, reviewer identity, and correlation
    /// values from the reachable typed IR instead of trusting model-authored capability prose.
    #[serde(skip_serializing_if = "Option::is_none")]
    approval_loop: Option<RequestApprovalLoopContract>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct RequestApprovalLoopContract {
    /// Exact addresses copied from reviewer/approval context in the immutable raw request.
    /// An empty list still requires one stable literal reviewer address in both review sends.
    reviewer_emails: Vec<String>,
    /// Approval may arrive as an inbound reviewer email or as two distinct page actions.
    /// These transports have different trust and dataflow boundaries: a page action must not be
    /// forced to invent an inbound sender/decision payload merely to satisfy the email contract.
    channel: RequestApprovalChannel,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum RequestApprovalChannel {
    EmailReply,
    PageAction,
}

/// Domain-separated digest of the immutable, host-supplied raw request. Model-authored tool JSON
/// can neither choose nor rewrite this value. Whitespace-only transport differences normalize to
/// the same identity; all other request text remains significant.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct FlowIrRequestIdentity(String);

impl FlowIrRequestIdentity {
    pub fn from_raw_request(raw_request: &str) -> Self {
        let normalized = normalize_raw_request_identity_input(raw_request);
        let mut hasher = blake3::Hasher::new();
        hasher.update(FLOW_IR_REQUEST_IDENTITY_DOMAIN);
        hasher.update(normalized.as_bytes());
        Self(format!("b3:{}", hasher.finalize().to_hex()))
    }

    fn unbound() -> Self {
        let mut hasher = blake3::Hasher::new();
        hasher.update(FLOW_IR_UNBOUND_IDENTITY_DOMAIN);
        Self(format!("b3:{}", hasher.finalize().to_hex()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn normalize_raw_request_identity_input(raw_request: &str) -> String {
    let normalized_line_endings = raw_request.replace("\r\n", "\n").replace('\r', "\n");
    normalized_line_endings
        .trim()
        .split('\n')
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug, thiserror::Error)]
#[error("FlowPilot typed IR tool error: {0}")]
pub struct FlowIrToolError(pub String);

/// One canonical repair hint shared by every typed-IR adapter. Keeping this next to the schemas
/// prevents the direct rig loop and SDK bridge from teaching different field/variant spellings
/// after a parse failure.
pub fn typed_ir_schema_hint() -> serde_json::Value {
    json!({
        "type_object": { "data_type": "string", "container": "normal", "interface": null },
        "primitive_data_types": ["string", "boolean", "integer", "float", "struct"],
        "literal_boolean": { "kind": "literal", "value": { "type": "boolean", "value": true } },
        "literal_integer": { "kind": "literal", "value": { "type": "integer", "value": 1 } },
        "value_ref": { "kind": "ref", "name": "customer_id" },
        "call_function_step": {
            "kind": "call_function",
            "id": "call_helper",
            "function": "helper",
            "args": []
        },
        "if_step": {
            "kind": "if",
            "id": "branch",
            "condition": { "kind": "ref", "name": "is_valid" },
            "then_steps": [],
            "else_steps": []
        },
        "capability_selection": {
            "discovery": { "id": "hash", "intent": "SHA256 hash", "required": true },
            "selected": {
                "id": "hash",
                "intent": "SHA256 hash",
                "required": true,
                "exact_node_type": "utils_hash_sha256"
            }
        },
        "rules": [
            "use canonical type objects with boolean and integer",
            "references use kind=ref; function calls use kind=call_function",
            "if bodies use then_steps and else_steps",
            "every required capability must select exact_node_type from a selection_required candidate before feasible can be true"
        ]
    })
}

pub fn render_typed_ir_parse_error(
    code: &str,
    context: &str,
    error: &impl std::fmt::Display,
) -> String {
    json!({
        "status": "validation_errors",
        "code": code,
        "message": format!("Failed to parse {context}: {error}"),
        "schema_hint": typed_ir_schema_hint()
    })
    .to_string()
}

#[derive(Debug, Clone)]
struct EvaluatedDraft {
    compile: FlowIrCompileResult,
    reconcile: Option<ReconcileResult>,
    /// Whole-workflow completeness is deferred while a draft is assembled module-by-module.
    /// Validate/commit promote these into hard diagnostics; begin/upsert expose only compact ids.
    completion_diagnostics: Vec<FlowIrDiagnostic>,
    diagnostics: Vec<FlowIrDiagnostic>,
}

/// Cached structural result for the retained revision. Draft assembly deliberately stops here:
/// capability coverage, host acceptance, and board reconciliation are whole-program gates and run
/// only from validate/commit. Keeping this cache also means a repair compares against the exact
/// retained structural state without compiling the current revision a second time.
#[derive(Debug, Clone, Serialize)]
struct StagedDraftEvaluation {
    compile: FlowIrCompileResult,
    diagnostics: Vec<FlowIrDiagnostic>,
}

impl StagedDraftEvaluation {
    fn diagnostic_count(&self) -> usize {
        self.diagnostics.len()
    }
}

#[derive(Debug, Clone, Serialize)]
struct CachedValidationSummary {
    revision: u64,
    diagnostics: Vec<FlowIrDiagnostic>,
    remaining_capabilities: Vec<String>,
    valid: bool,
}

impl EvaluatedDraft {
    fn is_valid(&self) -> bool {
        self.diagnostics.is_empty()
            && self.completion_diagnostics.is_empty()
            && self
                .reconcile
                .as_ref()
                .is_some_and(|result| !result.commands.is_empty())
    }

    fn complete(mut self, acceptance_diagnostics: Vec<FlowIrDiagnostic>) -> Self {
        self.diagnostics.append(&mut self.completion_diagnostics);
        self.diagnostics.extend(acceptance_diagnostics);
        self
    }
}

#[derive(Debug, Clone)]
struct StoredDraft {
    access_sequence: u64,
    /// Monotonic mutation generation used together with `revision` for ABA-safe compare-and-swap.
    /// A replacement begin can reset a draft id to revision zero, so revision alone is insufficient.
    state_sequence: u64,
    revision: u64,
    board_id: String,
    base_fingerprint: String,
    expected_modules: HashMap<String, FlowModuleKind>,
    capability_request: FlowCapabilityPlanRequest,
    /// Catalog resolution is expensive and immutable for an unchanged capability request. Draft
    /// operations reuse this snapshot; compilation still checks every node against the live
    /// catalog, so a catalog removal fails closed.
    capability_plan: FlowCapabilityPlan,
    /// Immutable host-derived requirements captured when this draft begins. Later chat turns may
    /// bind a different board-level contract for a new draft, but cannot reinterpret this one.
    request_acceptance_contract: RequestAcceptanceContract,
    /// Opaque identity of the immutable raw request that supplied the acceptance contract. This
    /// prevents a later, unrelated request on the same board from silently inheriting the draft.
    request_identity: FlowIrRequestIdentity,
    mode: FlowIrDraftMode,
    program: FlowIrProgram,
    staged_evaluation: StagedDraftEvaluation,
    validated: Option<CachedValidationSummary>,
    /// The best candidate is deliberately all-or-nothing: it must compile, reconcile, and cover
    /// every expected module independently of the mutable current draft.
    best: Option<(u64, FlowIrProgram)>,
    /// Claim the revision once commands have been returned so retries cannot enqueue them twice.
    /// A host that fails before queueing can explicitly release the claim.
    committed_revision: Option<u64>,
    /// Keep the board-scoped store pinned until the host explicitly acknowledges Apply or Dismiss.
    /// `committed_revision` remains as the permanent per-draft idempotency record after Apply.
    pending_revision: Option<u64>,
    /// Unique generation for the current pending delivery. A retry after Dismiss receives a new
    /// claim id so a duplicated/stale disposition cannot resolve the replacement batch (ABA).
    pending_claim_id: Option<String>,
    /// Exact typed-and-reconciled batch returned for the current pending delivery. Hosts apply
    /// this retained copy after revalidating the claim instead of trusting a client round-trip.
    pending_commands: Option<Vec<BoardCommand>>,
}

/// One evaluated source candidate. The parsed [`flow_like_ast::BoardAst`] is intentionally
/// ephemeral: FlowScript text is the model-authored artifact, while the AST remains an internal
/// compiler IR that can never be selected or mutated through model JSON.
#[derive(Debug, Clone)]
struct EvaluatedFlowScriptSource {
    diagnostics: Vec<FlowScriptDiagnostic>,
    /// Non-blocking acceptance findings. The acceptance projection is a heuristic that can
    /// false-positive on provably correct scripts, so incomplete-scope and approval-shape findings
    /// are surfaced to the human review instead of permanently blocking check/commit.
    review_notes: Vec<FlowScriptDiagnostic>,
    commands: Vec<BoardCommand>,
    corrections: Vec<String>,
}

impl EvaluatedFlowScriptSource {
    fn is_valid(&self) -> bool {
        self.diagnostics.is_empty() && !self.commands.is_empty()
    }
}

#[derive(Debug, Clone)]
struct RetainedFlowScriptCandidate {
    source: String,
    profile: FlowScriptCandidateProfile,
    parse_valid: bool,
    diagnostic_count: usize,
}

#[derive(Debug, Clone)]
struct CheckedFlowScriptRevision {
    revision: u64,
    board_fingerprint: String,
    /// Deterministic digest of the exact live catalog contract used to derive `commands`.
    /// Commit must observe the same digest so a pin/type/schema change cannot release a stale
    /// checked batch merely because the board itself remained unchanged.
    catalog_fingerprint: String,
    /// Exact compiler/reconciler output retained at check time. Commit moves this same batch into
    /// the pending claim; it never recompiles and silently substitutes a different command list.
    commands: Vec<BoardCommand>,
}

/// The last fully checked revision retained across later failed patches. It allows an explicit
/// commit to fall back to this exact source and command batch when the mutable head cannot be
/// repaired, instead of discarding a whole session of validated work.
#[derive(Debug, Clone)]
struct SalvageFlowScriptRevision {
    checked: CheckedFlowScriptRevision,
    source: String,
    evaluation: EvaluatedFlowScriptSource,
}

#[derive(Debug, Clone)]
struct StoredFlowScriptDraft {
    access_sequence: u64,
    /// Monotonic mutation generation used with `revision` for ABA-safe compare-and-swap.
    state_sequence: u64,
    revision: u64,
    board_id: String,
    base_fingerprint: String,
    request_acceptance_contract: RequestAcceptanceContract,
    request_identity: FlowIrRequestIdentity,
    mode: FlowIrDraftMode,
    source: String,
    evaluation: EvaluatedFlowScriptSource,
    /// Deterministic digest of the exact live catalog `evaluation` was computed against. A check
    /// at the same revision, board, and catalog returns this stored evaluation instead of running
    /// a redundant parse+reconcile.
    evaluation_catalog_fingerprint: String,
    best_candidate: RetainedFlowScriptCandidate,
    checked: Option<CheckedFlowScriptRevision>,
    salvage: Option<SalvageFlowScriptRevision>,
    committed_revision: Option<u64>,
    pending_revision: Option<u64>,
    pending_claim_id: Option<String>,
    pending_commands: Option<Vec<BoardCommand>>,
}

#[cfg(test)]
#[derive(Debug, Default)]
struct EvaluationTestGate {
    entered: Mutex<bool>,
    entered_cv: Condvar,
    released: Mutex<bool>,
    released_cv: Condvar,
}

#[cfg(test)]
impl EvaluationTestGate {
    fn enter_and_wait(&self) {
        if let Ok(mut entered) = self.entered.lock() {
            *entered = true;
            self.entered_cv.notify_all();
        }
        if let Ok(mut released) = self.released.lock() {
            while !*released {
                released = self
                    .released_cv
                    .wait(released)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
        }
    }

    fn wait_until_entered(&self, timeout: std::time::Duration) -> bool {
        if let Ok(mut entered) = self.entered.lock() {
            if !*entered {
                let (next, wait) = self
                    .entered_cv
                    .wait_timeout(entered, timeout)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                entered = next;
                if wait.timed_out() && !*entered {
                    return false;
                }
            }
            return *entered;
        }
        false
    }

    fn release(&self) {
        if let Ok(mut released) = self.released.lock() {
            *released = true;
            self.released_cv.notify_all();
        }
    }
}

#[cfg(test)]
#[derive(Debug, Default)]
struct EvaluationTestControl {
    global_evaluations: AtomicU64,
    pause_next_staged: Mutex<Option<Arc<EvaluationTestGate>>>,
}

#[derive(Debug, Clone)]
struct PendingRequestAcceptanceContract {
    board_id: String,
    contract: RequestAcceptanceContract,
    request_identity: FlowIrRequestIdentity,
    claimed_draft_id: Option<String>,
    access_sequence: u64,
}

/// Opaque host-side handle binding one user request to one draft session. It is never accepted in
/// model-authored JSON, which prevents the model from selecting another request's weaker contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowIrAcceptanceBinding {
    id: String,
    board_id: String,
    criterion_count: usize,
    request_identity: FlowIrRequestIdentity,
}

impl FlowIrAcceptanceBinding {
    pub fn criterion_count(&self) -> usize {
        self.criterion_count
    }

    pub fn request_identity(&self) -> &FlowIrRequestIdentity {
        &self.request_identity
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowIrDraftRecoveryStatus {
    None,
    ExactMatch,
    RequestMismatch,
}

/// Host-only recovery decision. Only `exact_match` is eligible for automatic continuation. A
/// conflicting draft is intentionally separated and accompanied by explicit user/host actions.
#[derive(Debug, Clone, Serialize)]
pub struct FlowIrDraftRecovery {
    pub status: FlowIrDraftRecoveryStatus,
    pub auto_resume: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exact_match: Option<FlowIrEditableDraftContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflicting_draft: Option<FlowIrEditableDraftContext>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub next_actions: Vec<String>,
    pub message: String,
}

impl FlowIrDraftRecovery {
    fn none() -> Self {
        Self {
            status: FlowIrDraftRecoveryStatus::None,
            auto_resume: false,
            exact_match: None,
            conflicting_draft: None,
            next_actions: Vec::new(),
            message: "No editable typed draft is retained for this board.".to_string(),
        }
    }
}

/// Host-only recovery payload for code-first drafts. Unlike the typed recovery summary, this
/// deliberately carries the exact retained source only when the immutable request identity
/// matches, so a timeout/new chat can resume editing without exposing another request's draft.
#[derive(Debug, Clone, Serialize)]
pub struct FlowScriptEditableDraftContext {
    pub board_id: String,
    pub draft_id: String,
    pub revision: u64,
    pub status: String,
    pub base_fingerprint: String,
    /// Present only for an exact request-identity match (or a direct host-only latest lookup).
    /// Conflicting requests receive no draft context at all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<FlowScriptDiagnostic>,
    pub checked: bool,
    pub stale_board: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FlowScriptDraftRecovery {
    pub status: FlowIrDraftRecoveryStatus,
    pub auto_resume: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exact_match: Option<FlowScriptEditableDraftContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflicting_draft: Option<FlowScriptEditableDraftContext>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub next_actions: Vec<String>,
    pub message: String,
}

/// Exact host-only payload for redelivering a FlowScript review whose original response may have
/// been lost. This is deliberately read-only: inspecting or dropping it never releases, replaces,
/// or reclaims the pending commit. The same nonce therefore survives any number of interrupted
/// redelivery attempts until the host applies or dismisses it through the normal disposition API.
#[derive(Debug, Clone)]
pub struct FlowScriptPendingDelivery {
    pub source: String,
    pub token: FlowIrCommitToken,
    /// A stale review is recoverable only so the host can surface its source and exact Dismiss
    /// token. Its command vector is always empty and native Apply still verifies the base.
    pub stale_board: bool,
    pub commands: Vec<BoardCommand>,
}

impl FlowScriptDraftRecovery {
    fn none() -> Self {
        Self {
            status: FlowIrDraftRecoveryStatus::None,
            auto_resume: false,
            exact_match: None,
            conflicting_draft: None,
            next_actions: Vec::new(),
            message: "No editable FlowScript source draft is retained for this board.".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FlowIrDraftRequestMismatch {
    pub status: &'static str,
    pub code: &'static str,
    pub retryable: bool,
    pub auto_resume: bool,
    /// Host-only coordinates used to classify recovery. Never disclose them to a differently
    /// bound model request.
    #[serde(skip_serializing)]
    pub draft_id: String,
    #[serde(skip_serializing)]
    pub revision: u64,
    pub next_actions: Vec<String>,
    pub message: String,
}

#[derive(Debug, Clone)]
struct ClaimedRequestAcceptanceContract {
    contract: RequestAcceptanceContract,
    request_identity: FlowIrRequestIdentity,
}

/// Per-chat, in-memory draft store. It intentionally contains no board mutations; only `commit`
/// returns commands, and the host keeps the existing review/approval boundary around those.
#[derive(Debug, Default)]
pub struct FlowIrDraftStore {
    drafts: Mutex<HashMap<String, StoredDraft>>,
    /// Code-first FlowScript sessions are retained separately from legacy typed-IR drafts, but
    /// share board-scoped commit claims and immutable request bindings.
    source_drafts: Mutex<HashMap<String, StoredFlowScriptDraft>>,
    request_acceptance_contracts: Mutex<HashMap<String, PendingRequestAcceptanceContract>>,
    access_sequence: AtomicU64,
    #[cfg(test)]
    evaluation_test_control: EvaluationTestControl,
}

impl FlowIrDraftStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn next_access_sequence(&self) -> u64 {
        self.access_sequence.fetch_add(1, Ordering::Relaxed)
    }

    fn evaluate_staged_program(
        &self,
        catalog: &[NodeMetadata],
        program: FlowIrProgram,
        expected_modules: &HashMap<String, FlowModuleKind>,
    ) -> StagedDraftEvaluation {
        #[cfg(test)]
        {
            let gate = self
                .evaluation_test_control
                .pause_next_staged
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take();
            if let Some(gate) = gate {
                gate.enter_and_wait();
            }
        }
        evaluate_staged_program(catalog, program, expected_modules)
    }

    #[allow(clippy::too_many_arguments)]
    fn evaluate_complete_program(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        program: FlowIrProgram,
        capability_request: &FlowCapabilityPlanRequest,
        capability_plan: &FlowCapabilityPlan,
        expected_modules: &HashMap<String, FlowModuleKind>,
        mode: FlowIrDraftMode,
    ) -> EvaluatedDraft {
        #[cfg(test)]
        self.evaluation_test_control
            .global_evaluations
            .fetch_add(1, Ordering::Relaxed);
        evaluate_program(
            board,
            catalog,
            program,
            capability_request,
            capability_plan,
            expected_modules,
            mode,
        )
    }

    fn evaluate_flowscript(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        source: &str,
        mode: FlowIrDraftMode,
        acceptance_contract: Option<&RequestAcceptanceContract>,
    ) -> EvaluatedFlowScriptSource {
        #[cfg(test)]
        self.evaluation_test_control
            .global_evaluations
            .fetch_add(1, Ordering::Relaxed);
        evaluate_flowscript_source(board, catalog, source, mode, acceptance_contract)
    }

    #[cfg(test)]
    fn global_evaluation_count(&self) -> u64 {
        self.evaluation_test_control
            .global_evaluations
            .load(Ordering::Relaxed)
    }

    #[cfg(test)]
    fn pause_next_staged_evaluation(&self) -> Arc<EvaluationTestGate> {
        let gate = Arc::new(EvaluationTestGate::default());
        *self
            .evaluation_test_control
            .pause_next_staged
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(gate.clone());
        gate
    }

    /// Bind a deterministic acceptance contract derived from the original user request. The
    /// returned opaque handle belongs to this request, not merely to its board: concurrent chats
    /// on one board therefore cannot consume or overwrite each other's scope.
    pub fn bind_request_acceptance_contract(
        &self,
        board_id: &str,
        prompt: &str,
    ) -> FlowIrAcceptanceBinding {
        let board_id = board_id.trim();
        let contract = derive_request_acceptance_contract(prompt);
        let criterion_count =
            contract.criteria.len() + usize::from(contract.approval_loop.is_some());
        let request_identity = FlowIrRequestIdentity::from_raw_request(prompt);
        let binding = FlowIrAcceptanceBinding {
            id: create_id(),
            board_id: board_id.to_string(),
            criterion_count,
            request_identity: request_identity.clone(),
        };
        if board_id.is_empty() {
            return binding;
        }
        let mut contracts = self
            .request_acceptance_contracts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        evict_pending_contract_capacity(&mut contracts);
        contracts.insert(
            binding.id.clone(),
            PendingRequestAcceptanceContract {
                board_id: board_id.to_string(),
                contract,
                request_identity,
                claimed_draft_id: None,
                access_sequence: self.next_access_sequence(),
            },
        );
        binding
    }

    /// Explicitly discard an unused request binding. This never alters a contract already copied
    /// into a StoredDraft and never clears another concurrent request on the same board.
    pub fn release_request_acceptance_contract(&self, binding: &FlowIrAcceptanceBinding) -> bool {
        self.request_acceptance_contracts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&binding.id)
            .is_some()
    }

    fn claim_request_acceptance_contract(
        &self,
        board_id: &str,
        draft_id: &str,
        binding: Option<&FlowIrAcceptanceBinding>,
    ) -> Result<ClaimedRequestAcceptanceContract, (&'static str, String)> {
        let Some(binding) = binding else {
            return Ok(ClaimedRequestAcceptanceContract {
                contract: RequestAcceptanceContract::default(),
                request_identity: FlowIrRequestIdentity::unbound(),
            });
        };
        if binding.board_id != board_id.trim() {
            return Err((
                "IR_ACCEPTANCE_BINDING_BOARD_MISMATCH",
                "the host acceptance binding belongs to a different board".to_string(),
            ));
        }
        let mut contracts = self
            .request_acceptance_contracts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(pending) = contracts.get_mut(&binding.id) else {
            return Err((
                "IR_ACCEPTANCE_BINDING_INVALID",
                "the host acceptance binding is missing, expired, or already consumed".to_string(),
            ));
        };
        if pending.board_id != board_id.trim() {
            return Err((
                "IR_ACCEPTANCE_BINDING_BOARD_MISMATCH",
                "the retained host acceptance contract belongs to a different board".to_string(),
            ));
        }
        if pending.request_identity != binding.request_identity {
            return Err((
                "IR_ACCEPTANCE_BINDING_IDENTITY_MISMATCH",
                "the host acceptance binding identity no longer matches its retained request"
                    .to_string(),
            ));
        }
        match pending.claimed_draft_id.as_deref() {
            Some(claimed) if claimed != draft_id => Err((
                "IR_ACCEPTANCE_BINDING_ALREADY_CLAIMED",
                format!("this host acceptance binding is already attached to draft {claimed:?}"),
            )),
            _ => {
                pending.claimed_draft_id = Some(draft_id.to_string());
                Ok(ClaimedRequestAcceptanceContract {
                    contract: pending.contract.clone(),
                    request_identity: pending.request_identity.clone(),
                })
            }
        }
    }

    /// A base-revision conflict is a dead end for the claimed source draft: the retained source
    /// stays available as reference, but every subsequent write/patch/check/commit on it fails and
    /// the model is told to start a new draft from the current board. Re-open the pending contract
    /// under its original binding so that fresh draft can claim the SAME immutable request scope
    /// within the same run. The identity checks keep a differently bound request from claiming it.
    fn reopen_request_acceptance_contract(
        &self,
        binding: Option<&FlowIrAcceptanceBinding>,
        draft: &StoredFlowScriptDraft,
    ) {
        self.reopen_request_acceptance_contract_scope(
            binding,
            &draft.board_id,
            &draft.request_acceptance_contract,
            &draft.request_identity,
        );
    }

    /// Typed-IR twin of [`Self::reopen_request_acceptance_contract`]: a typed draft that dead-ends
    /// on a base-revision conflict must hand its immutable request scope back to the binding so a
    /// fresh draft in the same run can claim it.
    fn reopen_typed_request_acceptance_contract(
        &self,
        binding: Option<&FlowIrAcceptanceBinding>,
        draft: &StoredDraft,
    ) {
        self.reopen_request_acceptance_contract_scope(
            binding,
            &draft.board_id,
            &draft.request_acceptance_contract,
            &draft.request_identity,
        );
    }

    fn reopen_request_acceptance_contract_scope(
        &self,
        binding: Option<&FlowIrAcceptanceBinding>,
        board_id: &str,
        contract: &RequestAcceptanceContract,
        request_identity: &FlowIrRequestIdentity,
    ) {
        let Some(binding) = binding else {
            return;
        };
        if binding.board_id != board_id || binding.request_identity != *request_identity {
            return;
        }
        let mut contracts = self
            .request_acceptance_contracts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let access_sequence = self.next_access_sequence();
        if let Some(pending) = contracts.get_mut(&binding.id) {
            if pending.board_id == binding.board_id
                && pending.request_identity == binding.request_identity
            {
                pending.claimed_draft_id = None;
                pending.access_sequence = access_sequence;
            }
            return;
        }
        evict_pending_contract_capacity(&mut contracts);
        contracts.insert(
            binding.id.clone(),
            PendingRequestAcceptanceContract {
                board_id: board_id.to_string(),
                contract: contract.clone(),
                request_identity: request_identity.clone(),
                claimed_draft_id: None,
                access_sequence,
            },
        );
    }

    /// A missing draft can still hold the claim on its pending request contract when the write
    /// that claimed it failed before retention (or the draft was evicted). Release exactly that
    /// claim so the same binding can start a fresh draft; the retained contract is unchanged.
    fn release_missing_draft_claim(
        &self,
        binding: Option<&FlowIrAcceptanceBinding>,
        draft_id: &str,
    ) {
        let Some(binding) = binding else {
            return;
        };
        let mut contracts = self
            .request_acceptance_contracts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(pending) = contracts.get_mut(&binding.id)
            && pending.claimed_draft_id.as_deref() == Some(draft_id)
        {
            pending.claimed_draft_id = None;
            pending.access_sequence = self.next_access_sequence();
        }
    }

    pub fn write_flowscript(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        args: WriteFlowScriptArgs,
    ) -> FlowScriptDraftResponse {
        self.write_flowscript_internal(board, catalog, args, None)
    }

    pub fn write_flowscript_with_acceptance_binding(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        args: WriteFlowScriptArgs,
        binding: &FlowIrAcceptanceBinding,
    ) -> FlowScriptDraftResponse {
        self.write_flowscript_internal(board, catalog, args, Some(binding))
    }

    fn write_flowscript_internal(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        args: WriteFlowScriptArgs,
        binding: Option<&FlowIrAcceptanceBinding>,
    ) -> FlowScriptDraftResponse {
        let draft_id = args.draft_id.trim().to_string();
        if draft_id.is_empty() {
            return FlowScriptDraftResponse::error(
                "FLOWSCRIPT_DRAFT_ID_REQUIRED",
                "draft_id must be non-empty",
            );
        }
        if draft_id.len() > MAX_FLOW_IR_DRAFT_ID_BYTES {
            return FlowScriptDraftResponse::error(
                "FLOWSCRIPT_DRAFT_ID_TOO_LONG",
                format!("draft_id must be at most {MAX_FLOW_IR_DRAFT_ID_BYTES} bytes"),
            );
        }
        if args.source.len() > MAX_FLOWSCRIPT_SOURCE_BYTES {
            return FlowScriptDraftResponse::error(
                "FLOWSCRIPT_SOURCE_SIZE_LIMIT_EXCEEDED",
                format!("source must be at most {MAX_FLOWSCRIPT_SOURCE_BYTES} bytes"),
            );
        }

        let base_fingerprint = board_fingerprint(board);

        let existing = self
            .source_drafts
            .lock()
            .ok()
            .and_then(|drafts| drafts.get(&draft_id).cloned());
        if let Some(existing) = &existing {
            if let Some(denied) =
                source_draft_request_authorization_error(&board.id, &draft_id, existing, binding)
            {
                return flowscript_request_mismatch_response(denied);
            }
            if existing.committed_revision.is_some() {
                let mut response = FlowScriptDraftResponse::for_draft(
                    "error",
                    "This draft already returned a command batch. Resolve it or use a new draft id.",
                    draft_id,
                    existing,
                );
                response.code = Some("FLOWSCRIPT_DRAFT_COMMIT_PENDING".to_string());
                return response;
            }
            if !args.replace_existing {
                let mut response = FlowScriptDraftResponse::for_draft(
                    "error",
                    "This draft id already has retained source. Patch its current revision, or explicitly replace it without changing request scope.",
                    draft_id,
                    existing,
                );
                response.code = Some("FLOWSCRIPT_DRAFT_ALREADY_EXISTS".to_string());
                return response;
            }
            if existing.base_fingerprint != base_fingerprint {
                self.reopen_request_acceptance_contract(binding, existing);
                return flowscript_base_revision_conflict_response(draft_id, existing);
            }
        }

        if self
            .drafts
            .lock()
            .is_ok_and(|drafts| drafts.contains_key(&draft_id))
        {
            return FlowScriptDraftResponse::error(
                "FLOWSCRIPT_DRAFT_ID_COLLISION",
                "draft_id is already used by a typed IR draft; choose a different id",
            );
        }

        let claimed_request = if let Some(existing) = &existing {
            ClaimedRequestAcceptanceContract {
                contract: existing.request_acceptance_contract.clone(),
                request_identity: existing.request_identity.clone(),
            }
        } else {
            match self.claim_request_acceptance_contract(&board.id, &draft_id, binding) {
                Ok(claimed) => claimed,
                Err((code, message)) => return FlowScriptDraftResponse::error(code, message),
            }
        };

        // The request contract is host-owned and deliberately absent from model arguments. Parse
        // and reconcile exactly once per submitted revision, directly against the claimed
        // contract; check_flowscript reuses this stored evaluation while board and catalog are
        // unchanged. BoardAst lives only inside this helper; neither the retained state nor the
        // model-visible response can smuggle it across turns.
        let evaluation = self.evaluate_flowscript(
            board,
            catalog,
            &args.source,
            args.mode,
            Some(&claimed_request.contract),
        );
        let candidate = retained_flowscript_candidate(&args.source, &evaluation);
        if let Some(existing) = &existing
            && !args.allow_scope_reduction
            && let Some(regression) = detect_flowscript_candidate_regression(
                &existing.best_candidate.profile,
                &candidate.profile,
            )
        {
            return flowscript_candidate_regression_response(draft_id, existing, regression);
        }

        let state_sequence = self.next_access_sequence();
        let revision = existing
            .as_ref()
            .map_or(0, |draft| draft.revision.saturating_add(1));
        let best_candidate = if args.allow_scope_reduction {
            candidate
        } else {
            existing
                .as_ref()
                .map(|draft| {
                    select_best_flowscript_candidate(&draft.best_candidate, candidate.clone())
                })
                .unwrap_or(candidate)
        };
        let stored = StoredFlowScriptDraft {
            access_sequence: state_sequence,
            state_sequence,
            revision,
            board_id: board.id.clone(),
            // A full rewrite inside one session must not silently rebase after an external board
            // edit. Only a new draft id captures a fresh base fingerprint.
            base_fingerprint: existing
                .as_ref()
                .map(|draft| draft.base_fingerprint.clone())
                .unwrap_or(base_fingerprint),
            request_acceptance_contract: claimed_request.contract,
            request_identity: claimed_request.request_identity,
            mode: args.mode,
            source: args.source,
            evaluation,
            evaluation_catalog_fingerprint: flowscript_catalog_fingerprint(catalog),
            best_candidate,
            checked: None,
            salvage: existing.as_ref().and_then(salvageable_flowscript_revision),
            committed_revision: None,
            pending_revision: None,
            pending_claim_id: None,
            pending_commands: None,
        };
        let stored_bytes = stored_flowscript_draft_size(&stored);

        // Lock order across the shared store is always typed -> source.
        let typed_drafts = match self.drafts.lock() {
            Ok(drafts) => drafts,
            Err(_) => {
                return FlowScriptDraftResponse::error(
                    "FLOWSCRIPT_DRAFT_STORE_UNAVAILABLE",
                    "draft store lock is unavailable",
                );
            }
        };
        if typed_drafts.contains_key(&draft_id) {
            return FlowScriptDraftResponse::error(
                "FLOWSCRIPT_DRAFT_ID_COLLISION",
                "draft_id was concurrently claimed by a typed IR draft; choose a different id",
            );
        }
        let mut drafts = match self.source_drafts.lock() {
            Ok(drafts) => drafts,
            Err(_) => {
                return FlowScriptDraftResponse::error(
                    "FLOWSCRIPT_DRAFT_STORE_UNAVAILABLE",
                    "FlowScript draft store lock is unavailable",
                );
            }
        };
        match (existing.as_ref(), drafts.get(&draft_id)) {
            (None, Some(current)) => {
                if let Some(denied) =
                    source_draft_request_authorization_error(&board.id, &draft_id, current, binding)
                {
                    return flowscript_request_mismatch_response(denied);
                }
                return FlowScriptDraftResponse::revision_conflict(
                    draft_id,
                    current.revision,
                    0,
                    current,
                );
            }
            (Some(snapshot), Some(current))
                if current.state_sequence != snapshot.state_sequence
                    || current.revision != snapshot.revision =>
            {
                if let Some(denied) =
                    source_draft_request_authorization_error(&board.id, &draft_id, current, binding)
                {
                    return flowscript_request_mismatch_response(denied);
                }
                return FlowScriptDraftResponse::revision_conflict(
                    draft_id,
                    current.revision,
                    snapshot.revision,
                    current,
                );
            }
            (Some(_), None) => {
                return FlowScriptDraftResponse::error(
                    "FLOWSCRIPT_DRAFT_MISSING",
                    "retained source disappeared while replacement was being evaluated",
                );
            }
            _ => {}
        }
        loop {
            let other_count = drafts.keys().filter(|id| *id != &draft_id).count();
            let other_bytes = drafts
                .iter()
                .filter(|(id, _)| *id != &draft_id)
                .map(|(_, draft)| stored_flowscript_draft_size(draft))
                .fold(0usize, usize::saturating_add);
            if other_count < MAX_FLOWSCRIPT_DRAFTS_PER_STORE
                && other_bytes.saturating_add(stored_bytes) <= MAX_FLOWSCRIPT_DRAFT_STORE_BYTES
            {
                break;
            }
            let victim = drafts
                .iter()
                .filter(|(id, draft)| *id != &draft_id && draft.pending_revision.is_none())
                .min_by_key(|(_, draft)| draft.access_sequence)
                .map(|(id, _)| id.clone());
            let Some(victim) = victim else {
                return FlowScriptDraftResponse::error(
                    "FLOWSCRIPT_DRAFT_STORE_SIZE_LIMIT_EXCEEDED",
                    "retained FlowScript draft budget is exhausted by pending commits",
                );
            };
            drafts.remove(&victim);
        }
        drafts.insert(draft_id.clone(), stored.clone());
        drop(drafts);
        drop(typed_drafts);
        if existing.is_none()
            && let Some(binding) = binding
        {
            self.request_acceptance_contracts
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&binding.id);
        }
        let status = if stored.evaluation.diagnostics.is_empty() {
            "draft_started"
        } else {
            "validation_errors"
        };
        FlowScriptDraftResponse::for_draft(
            status,
            if stored.evaluation.diagnostics.is_empty() {
                "FlowScript source is retained. Run check_flowscript at this revision before commit."
            } else {
                "FlowScript source is retained with structured diagnostics. Patch this exact revision in place."
            },
            draft_id,
            &stored,
        )
    }

    pub fn patch_flowscript(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        args: PatchFlowScriptArgs,
    ) -> FlowScriptDraftResponse {
        self.patch_flowscript_internal(board, catalog, args, None)
    }

    pub fn patch_flowscript_with_acceptance_binding(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        args: PatchFlowScriptArgs,
        binding: &FlowIrAcceptanceBinding,
    ) -> FlowScriptDraftResponse {
        self.patch_flowscript_internal(board, catalog, args, Some(binding))
    }

    fn patch_flowscript_internal(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        args: PatchFlowScriptArgs,
        binding: Option<&FlowIrAcceptanceBinding>,
    ) -> FlowScriptDraftResponse {
        let draft_key = args.draft_id.trim();
        let current_board_fingerprint = board_fingerprint(board);
        let snapshot = match self.source_drafts.lock() {
            Ok(mut drafts) => {
                let Some(draft) = drafts.get_mut(draft_key) else {
                    drop(drafts);
                    self.release_missing_draft_claim(binding, draft_key);
                    return FlowScriptDraftResponse::error(
                        "FLOWSCRIPT_DRAFT_MISSING",
                        "FlowScript draft does not exist",
                    );
                };
                if let Some(denied) =
                    source_draft_request_authorization_error(&board.id, draft_key, draft, binding)
                {
                    return flowscript_request_mismatch_response(denied);
                }
                if draft.base_fingerprint != current_board_fingerprint {
                    self.reopen_request_acceptance_contract(binding, draft);
                    return flowscript_base_revision_conflict_response(
                        draft_key.to_string(),
                        draft,
                    );
                }
                draft.access_sequence = self.next_access_sequence();
                draft.clone()
            }
            Err(_) => {
                return FlowScriptDraftResponse::error(
                    "FLOWSCRIPT_DRAFT_STORE_UNAVAILABLE",
                    "FlowScript draft store lock is unavailable",
                );
            }
        };
        if snapshot.revision != args.expected_revision {
            return FlowScriptDraftResponse::revision_conflict(
                args.draft_id,
                snapshot.revision,
                args.expected_revision,
                &snapshot,
            );
        }
        if snapshot.committed_revision.is_some() {
            let mut response = FlowScriptDraftResponse::for_draft(
                "error",
                "This revision already returned a command batch and cannot be patched.",
                args.draft_id,
                &snapshot,
            );
            response.code = Some("FLOWSCRIPT_DRAFT_COMMIT_PENDING".to_string());
            return response;
        }
        if args.old_text.is_empty() {
            let mut response = FlowScriptDraftResponse::for_draft(
                "error",
                "old_text must be non-empty and identify one exact source range",
                args.draft_id,
                &snapshot,
            );
            response.code = Some("FLOWSCRIPT_PATCH_TEXT_REQUIRED".to_string());
            return response;
        }
        let occurrences = snapshot.source.match_indices(&args.old_text).count();
        if occurrences != 1 {
            let mut response = FlowScriptDraftResponse::for_draft(
                "error",
                format!(
                    "old_text must occur exactly once in revision {}; found {occurrences}",
                    snapshot.revision
                ),
                args.draft_id,
                &snapshot,
            );
            response.code = Some("FLOWSCRIPT_PATCH_NOT_UNIQUE".to_string());
            return response;
        }
        let candidate_source = snapshot.source.replacen(&args.old_text, &args.new_text, 1);
        if candidate_source.len() > MAX_FLOWSCRIPT_SOURCE_BYTES {
            let mut response = FlowScriptDraftResponse::for_draft(
                "error",
                format!("patched source exceeds {MAX_FLOWSCRIPT_SOURCE_BYTES} bytes"),
                args.draft_id,
                &snapshot,
            );
            response.code = Some("FLOWSCRIPT_SOURCE_SIZE_LIMIT_EXCEEDED".to_string());
            return response;
        }
        let evaluation = self.evaluate_flowscript(
            board,
            catalog,
            &candidate_source,
            snapshot.mode,
            Some(&snapshot.request_acceptance_contract),
        );
        let candidate = retained_flowscript_candidate(&candidate_source, &evaluation);
        if !args.allow_scope_reduction
            && let Some(regression) = detect_flowscript_candidate_regression(
                &snapshot.best_candidate.profile,
                &candidate.profile,
            )
        {
            return flowscript_candidate_regression_response(args.draft_id, &snapshot, regression);
        }

        let mut drafts = match self.source_drafts.lock() {
            Ok(drafts) => drafts,
            Err(_) => {
                return FlowScriptDraftResponse::error(
                    "FLOWSCRIPT_DRAFT_STORE_UNAVAILABLE",
                    "FlowScript draft store lock is unavailable",
                );
            }
        };
        let Some(current) = drafts.get(draft_key).cloned() else {
            drop(drafts);
            self.release_missing_draft_claim(binding, draft_key);
            return FlowScriptDraftResponse::error(
                "FLOWSCRIPT_DRAFT_MISSING",
                "FlowScript draft disappeared while the patch was evaluated",
            );
        };
        if let Some(denied) =
            source_draft_request_authorization_error(&board.id, draft_key, &current, binding)
        {
            return flowscript_request_mismatch_response(denied);
        }
        if current.base_fingerprint != current_board_fingerprint {
            self.reopen_request_acceptance_contract(binding, &current);
            return flowscript_base_revision_conflict_response(args.draft_id, &current);
        }
        if current.revision != snapshot.revision
            || current.state_sequence != snapshot.state_sequence
        {
            return FlowScriptDraftResponse::revision_conflict(
                args.draft_id,
                current.revision,
                args.expected_revision,
                &current,
            );
        }
        let mut prospective = current.clone();
        prospective.revision = prospective.revision.saturating_add(1);
        prospective.source = candidate_source;
        prospective.evaluation = evaluation;
        prospective.evaluation_catalog_fingerprint = flowscript_catalog_fingerprint(catalog);
        prospective.best_candidate = if args.allow_scope_reduction {
            candidate
        } else {
            select_best_flowscript_candidate(&snapshot.best_candidate, candidate)
        };
        // A patch invalidates the head check, but the last fully checked revision stays
        // salvageable: an explicit commit at that exact revision can still release its batch.
        prospective.salvage = salvageable_flowscript_revision(&current);
        prospective.checked = None;
        let state_sequence = self.next_access_sequence();
        prospective.state_sequence = state_sequence;
        prospective.access_sequence = state_sequence;
        let other_bytes = drafts
            .iter()
            .filter(|(id, _)| id.as_str() != draft_key)
            .map(|(_, draft)| stored_flowscript_draft_size(draft))
            .fold(0usize, usize::saturating_add);
        if other_bytes.saturating_add(stored_flowscript_draft_size(&prospective))
            > MAX_FLOWSCRIPT_DRAFT_STORE_BYTES
        {
            let mut response = FlowScriptDraftResponse::for_draft(
                "error",
                "The patched source and derived diagnostics/commands exceed the retained FlowScript draft budget.",
                args.draft_id,
                &current,
            );
            response.code = Some("FLOWSCRIPT_DRAFT_STORE_SIZE_LIMIT_EXCEEDED".to_string());
            return response;
        }
        drafts.insert(draft_key.to_string(), prospective.clone());
        let retained = prospective;
        let status = if retained.evaluation.diagnostics.is_empty() {
            "draft_updated"
        } else {
            "validation_errors"
        };
        FlowScriptDraftResponse::for_draft(
            status,
            if retained.evaluation.diagnostics.is_empty() {
                "Patch retained. Run check_flowscript at this revision before commit."
            } else {
                "Patch retained with structured diagnostics; repair this same source revision."
            },
            args.draft_id,
            &retained,
        )
    }

    pub fn check_flowscript(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        args: CheckFlowScriptArgs,
    ) -> FlowScriptDraftResponse {
        self.check_flowscript_internal(board, catalog, args, None)
    }

    pub fn check_flowscript_with_acceptance_binding(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        args: CheckFlowScriptArgs,
        binding: &FlowIrAcceptanceBinding,
    ) -> FlowScriptDraftResponse {
        self.check_flowscript_internal(board, catalog, args, Some(binding))
    }

    fn check_flowscript_internal(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        args: CheckFlowScriptArgs,
        binding: Option<&FlowIrAcceptanceBinding>,
    ) -> FlowScriptDraftResponse {
        let draft_key = args.draft_id.trim();
        let snapshot = match self.source_drafts.lock() {
            Ok(drafts) => {
                let Some(draft) = drafts.get(draft_key) else {
                    drop(drafts);
                    self.release_missing_draft_claim(binding, draft_key);
                    return FlowScriptDraftResponse::error(
                        "FLOWSCRIPT_DRAFT_MISSING",
                        "FlowScript draft does not exist",
                    );
                };
                if let Some(denied) =
                    source_draft_request_authorization_error(&board.id, draft_key, draft, binding)
                {
                    return flowscript_request_mismatch_response(denied);
                }
                draft.clone()
            }
            Err(_) => {
                return FlowScriptDraftResponse::error(
                    "FLOWSCRIPT_DRAFT_STORE_UNAVAILABLE",
                    "FlowScript draft store lock is unavailable",
                );
            }
        };
        if snapshot.revision != args.expected_revision {
            return FlowScriptDraftResponse::revision_conflict(
                args.draft_id,
                snapshot.revision,
                args.expected_revision,
                &snapshot,
            );
        }
        let current_fingerprint = board_fingerprint(board);
        if current_fingerprint != snapshot.base_fingerprint {
            self.reopen_request_acceptance_contract(binding, &snapshot);
            return flowscript_base_revision_conflict_response(args.draft_id, &snapshot);
        }
        // The stored evaluation was computed against this exact source, board revision, and
        // request contract. Re-run parse+reconcile only when the live catalog moved since then.
        let catalog_fingerprint = flowscript_catalog_fingerprint(catalog);
        let evaluation = if snapshot.evaluation_catalog_fingerprint == catalog_fingerprint {
            snapshot.evaluation.clone()
        } else {
            self.evaluate_flowscript(
                board,
                catalog,
                &snapshot.source,
                snapshot.mode,
                Some(&snapshot.request_acceptance_contract),
            )
        };
        let mut drafts = match self.source_drafts.lock() {
            Ok(drafts) => drafts,
            Err(_) => {
                return FlowScriptDraftResponse::error(
                    "FLOWSCRIPT_DRAFT_STORE_UNAVAILABLE",
                    "FlowScript draft store lock is unavailable",
                );
            }
        };
        let Some(current) = drafts.get(draft_key).cloned() else {
            drop(drafts);
            self.release_missing_draft_claim(binding, draft_key);
            return FlowScriptDraftResponse::error(
                "FLOWSCRIPT_DRAFT_MISSING",
                "FlowScript draft disappeared while check was running",
            );
        };
        if let Some(denied) =
            source_draft_request_authorization_error(&board.id, draft_key, &current, binding)
        {
            return flowscript_request_mismatch_response(denied);
        }
        if current.base_fingerprint != current_fingerprint {
            self.reopen_request_acceptance_contract(binding, &current);
            return flowscript_base_revision_conflict_response(args.draft_id, &current);
        }
        if current.revision != snapshot.revision
            || current.state_sequence != snapshot.state_sequence
        {
            return FlowScriptDraftResponse::revision_conflict(
                args.draft_id,
                current.revision,
                args.expected_revision,
                &current,
            );
        }
        let mut prospective = current.clone();
        prospective.evaluation = evaluation;
        prospective.evaluation_catalog_fingerprint = catalog_fingerprint.clone();
        prospective.checked =
            prospective
                .evaluation
                .is_valid()
                .then(|| CheckedFlowScriptRevision {
                    revision: prospective.revision,
                    board_fingerprint: current_fingerprint,
                    catalog_fingerprint,
                    commands: prospective.evaluation.commands.clone(),
                });
        if prospective.checked.is_some() {
            // A successful head check supersedes any older salvageable revision.
            prospective.salvage = None;
        }
        let state_sequence = self.next_access_sequence();
        prospective.state_sequence = state_sequence;
        prospective.access_sequence = state_sequence;
        let other_bytes = drafts
            .iter()
            .filter(|(id, _)| id.as_str() != draft_key)
            .map(|(_, draft)| stored_flowscript_draft_size(draft))
            .fold(0usize, usize::saturating_add);
        if other_bytes.saturating_add(stored_flowscript_draft_size(&prospective))
            > MAX_FLOWSCRIPT_DRAFT_STORE_BYTES
        {
            let mut response = FlowScriptDraftResponse::for_draft(
                "error",
                "The exact checked command batch exceeds the retained FlowScript draft budget.",
                args.draft_id,
                &current,
            );
            response.code = Some("FLOWSCRIPT_DRAFT_STORE_SIZE_LIMIT_EXCEEDED".to_string());
            return response;
        }
        drafts.insert(draft_key.to_string(), prospective.clone());
        let retained = prospective;
        if !retained.evaluation.diagnostics.is_empty() {
            return FlowScriptDraftResponse::for_draft(
                "validation_errors",
                "FlowScript check failed. Apply a unique text patch to this retained revision.",
                args.draft_id,
                &retained,
            );
        }
        if retained.evaluation.commands.is_empty() {
            let mut response = FlowScriptDraftResponse::for_draft(
                "no_changes",
                "FlowScript is valid but derives no board changes.",
                args.draft_id,
                &retained,
            );
            response.code = Some("FLOWSCRIPT_NO_CHANGES".to_string());
            return response;
        }
        let mut message = if retained.evaluation.review_notes.is_empty() {
            "FlowScript is valid and its exact command batch is retained for commit.".to_string()
        } else {
            format!(
                "FlowScript is valid and its exact command batch is retained for commit. Commit may proceed; {} non-blocking acceptance review note(s) will be surfaced in the human review.",
                retained.evaluation.review_notes.len()
            )
        };
        append_omitted_prohibition_notice(&mut message, &retained.request_acceptance_contract);
        FlowScriptDraftResponse::for_draft("valid", message, args.draft_id, &retained)
    }

    pub fn commit_flowscript(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        args: CommitFlowScriptArgs,
    ) -> FlowScriptDraftResponse {
        self.commit_flowscript_internal(board, catalog, args, None)
    }

    pub fn commit_flowscript_with_acceptance_binding(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        args: CommitFlowScriptArgs,
        binding: &FlowIrAcceptanceBinding,
    ) -> FlowScriptDraftResponse {
        self.commit_flowscript_internal(board, catalog, args, Some(binding))
    }

    fn commit_flowscript_internal(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        args: CommitFlowScriptArgs,
        binding: Option<&FlowIrAcceptanceBinding>,
    ) -> FlowScriptDraftResponse {
        let removal_ids = args
            .remove_node_ids
            .iter()
            .chain(&args.remove_variable_ids)
            .chain(&args.remove_layer_ids)
            .chain(&args.remove_comment_ids);
        if removal_ids.clone().count() > MAX_FLOW_IR_ENTITY_ALLOWLIST_ITEMS {
            return FlowScriptDraftResponse::error(
                "FLOWSCRIPT_DELETION_ALLOWLIST_LIMIT_EXCEEDED",
                format!(
                    "a commit may authorize at most {MAX_FLOW_IR_ENTITY_ALLOWLIST_ITEMS} removals"
                ),
            );
        }
        if removal_ids
            .clone()
            .any(|id| id.len() > MAX_FLOW_IR_AUTHORED_NAME_BYTES)
        {
            return FlowScriptDraftResponse::error(
                "FLOWSCRIPT_DELETION_ID_TOO_LONG",
                format!("deletion ids must be at most {MAX_FLOW_IR_AUTHORED_NAME_BYTES} bytes"),
            );
        }

        // Atomic board-level claim: lock order is typed -> source everywhere that needs both.
        let typed_drafts = match self.drafts.lock() {
            Ok(drafts) => drafts,
            Err(_) => {
                return FlowScriptDraftResponse::error(
                    "FLOWSCRIPT_DRAFT_STORE_UNAVAILABLE",
                    "draft store lock is unavailable",
                );
            }
        };
        let mut drafts = match self.source_drafts.lock() {
            Ok(drafts) => drafts,
            Err(_) => {
                return FlowScriptDraftResponse::error(
                    "FLOWSCRIPT_DRAFT_STORE_UNAVAILABLE",
                    "FlowScript draft store lock is unavailable",
                );
            }
        };
        let draft_key = args.draft_id.trim();
        let Some(mut snapshot) = drafts.get(draft_key).cloned() else {
            drop(drafts);
            drop(typed_drafts);
            self.release_missing_draft_claim(binding, draft_key);
            return FlowScriptDraftResponse::error(
                "FLOWSCRIPT_DRAFT_MISSING",
                "FlowScript draft does not exist",
            );
        };
        if let Some(denied) =
            source_draft_request_authorization_error(&board.id, draft_key, &snapshot, binding)
        {
            return flowscript_request_mismatch_response(denied);
        }
        let mut restored_from_salvage = false;
        if snapshot.revision != args.expected_revision {
            let salvage_matches = snapshot.committed_revision.is_none()
                && snapshot.pending_revision.is_none()
                && snapshot.salvage.as_ref().is_some_and(|salvage| {
                    salvage.checked.revision == args.expected_revision
                        && salvage.checked.board_fingerprint == snapshot.base_fingerprint
                });
            if !salvage_matches {
                return FlowScriptDraftResponse::revision_conflict(
                    args.draft_id,
                    snapshot.revision,
                    args.expected_revision,
                    &snapshot,
                );
            }
            // An explicit commit at the retained checked revision abandons the moved head and
            // restores that exact source/evaluation/batch. Board and catalog fingerprints are
            // still verified below, so this never releases stale commands.
            let salvage = snapshot
                .salvage
                .take()
                .expect("salvage presence was just verified");
            snapshot.revision = salvage.checked.revision;
            snapshot.source = salvage.source;
            snapshot.evaluation_catalog_fingerprint = salvage.checked.catalog_fingerprint.clone();
            snapshot.evaluation = salvage.evaluation;
            snapshot.checked = Some(salvage.checked);
            restored_from_salvage = true;
        }
        if snapshot.committed_revision == Some(snapshot.revision) {
            let mut response = FlowScriptDraftResponse::for_draft(
                "already_queued",
                "This exact revision already returned its retained command batch; no duplicate was queued.",
                args.draft_id,
                &snapshot,
            );
            response.code = Some("FLOWSCRIPT_DRAFT_ALREADY_COMMITTED".to_string());
            return response;
        }
        if board_fingerprint(board) != snapshot.base_fingerprint {
            self.reopen_request_acceptance_contract(binding, &snapshot);
            return flowscript_base_revision_conflict_response(args.draft_id, &snapshot);
        }
        let Some(checked) = snapshot.checked.as_ref().filter(|checked| {
            checked.revision == snapshot.revision
                && checked.board_fingerprint == snapshot.base_fingerprint
        }) else {
            let mut response = FlowScriptDraftResponse::for_draft(
                "error",
                "Run check_flowscript successfully at this exact revision before commit.",
                args.draft_id,
                &snapshot,
            );
            response.code = Some("FLOWSCRIPT_CHECK_REQUIRED".to_string());
            return response;
        };
        if checked.catalog_fingerprint != flowscript_catalog_fingerprint(catalog) {
            let mut response = FlowScriptDraftResponse::for_draft(
                "error",
                "The live catalog changed after this revision was checked. Run check_flowscript again at this exact revision before commit.",
                args.draft_id,
                &snapshot,
            );
            response.code = Some("FLOWSCRIPT_CATALOG_REVISION_CONFLICT".to_string());
            return response;
        }
        if typed_drafts.values().any(|draft| {
            draft.pending_revision.is_some() && draft.base_fingerprint == snapshot.base_fingerprint
        }) || drafts.iter().any(|(id, draft)| {
            id.as_str() != draft_key
                && draft.pending_revision.is_some()
                && draft.base_fingerprint == snapshot.base_fingerprint
        }) {
            let mut response = FlowScriptDraftResponse::for_draft(
                "error",
                "Another draft from this board revision already owns a pending command batch.",
                args.draft_id,
                &snapshot,
            );
            response.code = Some("FLOWSCRIPT_BOARD_COMMIT_PENDING".to_string());
            return response;
        }
        if let Some(response) =
            validate_flowscript_deletion_authorization(&snapshot, checked, &args)
        {
            return response;
        }

        let commands = checked.commands.clone();
        let claim_id = create_id();
        let mut prospective = snapshot.clone();
        prospective.committed_revision = Some(prospective.revision);
        prospective.pending_revision = Some(prospective.revision);
        prospective.pending_claim_id = Some(claim_id);
        prospective.pending_commands = Some(commands.clone());
        let state_sequence = self.next_access_sequence();
        prospective.state_sequence = state_sequence;
        prospective.access_sequence = state_sequence;
        let other_bytes = drafts
            .iter()
            .filter(|(id, _)| id.as_str() != draft_key)
            .map(|(_, draft)| stored_flowscript_draft_size(draft))
            .fold(0usize, usize::saturating_add);
        if other_bytes.saturating_add(stored_flowscript_draft_size(&prospective))
            > MAX_FLOWSCRIPT_DRAFT_STORE_BYTES
        {
            let mut response = FlowScriptDraftResponse::for_draft(
                "error",
                "The exact pending command claim exceeds the retained FlowScript draft budget; no claim was created.",
                args.draft_id,
                &snapshot,
            );
            response.code = Some("FLOWSCRIPT_DRAFT_STORE_SIZE_LIMIT_EXCEEDED".to_string());
            return response;
        }
        drafts.insert(draft_key.to_string(), prospective.clone());
        let retained = prospective;
        let mut message = format!(
            "Checked FlowScript queued {} exact atomic board change(s).",
            commands.len()
        );
        if restored_from_salvage {
            message.push_str(&format!(
                " The retained checked revision {} was restored; later unchecked head edits were discarded.",
                retained.revision
            ));
        }
        if !retained.evaluation.review_notes.is_empty() {
            message.push_str(&format!(
                " {} non-blocking acceptance review note(s) accompany this batch for the human review.",
                retained.evaluation.review_notes.len()
            ));
        }
        append_omitted_prohibition_notice(&mut message, &retained.request_acceptance_contract);
        let mut response =
            FlowScriptDraftResponse::for_draft("queued", message, args.draft_id, &retained);
        response.diagnostics.clear();
        response.queued_count = commands.len();
        response.commands = commands;
        response
    }

    pub fn begin(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        args: BeginFlowIrDraftArgs,
    ) -> FlowIrDraftResponse {
        self.begin_internal(board, catalog, args, None)
    }

    pub fn begin_with_acceptance_binding(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        args: BeginFlowIrDraftArgs,
        binding: &FlowIrAcceptanceBinding,
    ) -> FlowIrDraftResponse {
        self.begin_internal(board, catalog, args, Some(binding))
    }

    fn begin_internal(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        args: BeginFlowIrDraftArgs,
        acceptance_binding: Option<&FlowIrAcceptanceBinding>,
    ) -> FlowIrDraftResponse {
        let draft_id = args.draft_id.trim().to_string();
        if draft_id.is_empty() {
            return FlowIrDraftResponse::error(
                "IR_DRAFT_ID_REQUIRED",
                "draft_id must be non-empty",
            );
        }
        if draft_id.len() > MAX_FLOW_IR_DRAFT_ID_BYTES {
            return FlowIrDraftResponse::error(
                "IR_DRAFT_ID_TOO_LONG",
                format!("draft_id must be at most {MAX_FLOW_IR_DRAFT_ID_BYTES} bytes"),
            );
        }
        if args.expected_modules.len() > super::ir::MAX_FLOW_IR_MODULES {
            return FlowIrDraftResponse::error(
                "IR_EXPECTED_MODULE_LIMIT_EXCEEDED",
                format!(
                    "expected_modules may contain at most {} entries",
                    super::ir::MAX_FLOW_IR_MODULES
                ),
            );
        }
        if args
            .expected_modules
            .iter()
            .any(|name| name.len() > MAX_FLOW_IR_AUTHORED_NAME_BYTES)
        {
            return FlowIrDraftResponse::error(
                "IR_AUTHORED_NAME_TOO_LONG",
                format!(
                    "expected module names must be at most {MAX_FLOW_IR_AUTHORED_NAME_BYTES} bytes"
                ),
            );
        }
        if let Some((code, message)) = capability_request_limit_error(&args.capability_plan) {
            return FlowIrDraftResponse::error(code, message);
        }
        if serde_json::to_vec(&args.capability_plan)
            .map(|encoded| encoded.len() > MAX_FLOW_IR_CAPABILITY_PLAN_BYTES)
            .unwrap_or(true)
        {
            return FlowIrDraftResponse::error(
                "IR_CAPABILITY_PLAN_SIZE_LIMIT_EXCEEDED",
                format!(
                    "capability plan must be at most {MAX_FLOW_IR_CAPABILITY_PLAN_BYTES} serialized bytes"
                ),
            );
        }
        if capability_plan_has_oversized_names(&args.capability_plan) {
            return FlowIrDraftResponse::error(
                "IR_AUTHORED_NAME_TOO_LONG",
                format!(
                    "capability ids, node/pin names, module names, and scopes must be at most {MAX_FLOW_IR_AUTHORED_NAME_BYTES} bytes"
                ),
            );
        }
        if args.capability_plan.requirements.len() > MAX_FLOW_IR_CAPABILITY_REQUIREMENTS {
            return FlowIrDraftResponse::error(
                "IR_CAPABILITY_REQUIREMENT_LIMIT_EXCEEDED",
                format!(
                    "capability_plan may contain at most {MAX_FLOW_IR_CAPABILITY_REQUIREMENTS} requirements"
                ),
            );
        }
        if args.capability_plan.requirements.iter().any(|requirement| {
            requirement.inputs.len() > MAX_FLOW_IR_PIN_REQUIREMENTS_PER_DIRECTION
                || requirement.outputs.len() > MAX_FLOW_IR_PIN_REQUIREMENTS_PER_DIRECTION
        }) {
            return FlowIrDraftResponse::error(
                "IR_CAPABILITY_PIN_REQUIREMENT_LIMIT_EXCEEDED",
                format!(
                    "each capability may contain at most {MAX_FLOW_IR_PIN_REQUIREMENTS_PER_DIRECTION} input and output pin requirements"
                ),
            );
        }
        if args.capability_plan.modules.len() > super::ir::MAX_FLOW_IR_MODULES {
            return FlowIrDraftResponse::error(
                "IR_CAPABILITY_MODULE_LIMIT_EXCEEDED",
                format!(
                    "capability_plan may estimate at most {} modules",
                    super::ir::MAX_FLOW_IR_MODULES
                ),
            );
        }
        let expected_modules =
            match expected_module_contract(&args.expected_modules, &args.capability_plan) {
                Ok(expected) => expected,
                Err((code, message)) => return FlowIrDraftResponse::error(code, message),
            };
        if expected_modules.is_empty() {
            return FlowIrDraftResponse::error(
                "IR_EXPECTED_MODULES_REQUIRED",
                "expected_modules must name every required function and event module",
            );
        }
        if !args
            .capability_plan
            .requirements
            .iter()
            .any(|requirement| requirement.required)
        {
            return FlowIrDraftResponse::error(
                "IR_CAPABILITY_PLAN_REQUIRED",
                "capability_plan must include at least one required catalog capability produced by plan_flow_ir",
            );
        }
        let resource_diagnostics = validate_ir_resource_limits(&args.program);
        if !resource_diagnostics.is_empty() {
            let mut response = FlowIrDraftResponse::error(
                "IR_RESOURCE_LIMIT_EXCEEDED",
                "typed program exceeds a hard resource limit and was not retained",
            );
            response.diagnostics = resource_diagnostics;
            return response;
        }
        // Capability resolution and structural compilation can be expensive on a large live
        // catalog. They intentionally run before the draft-store critical section.
        let capability_plan = plan_flow_capabilities(&args.capability_plan, catalog);
        let staged_evaluation =
            self.evaluate_staged_program(catalog, args.program.clone(), &expected_modules);
        let base_fingerprint = board_fingerprint(board);
        let missing = missing_modules_for_program(&expected_modules, &args.program);
        if !capability_plan.feasible {
            return FlowIrDraftResponse {
                status: "infeasible".to_string(),
                code: Some("IR_CAPABILITY_PLAN_INFEASIBLE".to_string()),
                message: "Required catalog capabilities or module budgets are unavailable. The draft was not started; report the exact missing contract instead of generating a substitute."
                    .to_string(),
                draft_id: Some(draft_id),
                revision: None,
                base_fingerprint: Some(base_fingerprint),
                diagnostics: capability_plan.module_budget_violations.clone(),
                module_node_counts: staged_evaluation.compile.module_node_counts,
                flowscript: None,
                retained_ir: None,
                capability_plan: Some(capability_plan),
                remaining_capabilities: pending_required_capability_ids(&args.capability_plan),
                missing_modules: missing,
                derived_command_count: None,
                commands: Vec::new(),
            };
        }
        let claimed_request = match self.claim_request_acceptance_contract(
            &board.id,
            &draft_id,
            acceptance_binding,
        ) {
            Ok(contract) => contract,
            Err((code, message)) => return FlowIrDraftResponse::error(code, message),
        };
        let pending_capabilities = pending_required_capability_ids(&args.capability_plan);
        let state_sequence = self.next_access_sequence();
        let stored = StoredDraft {
            access_sequence: state_sequence,
            state_sequence,
            revision: 0,
            board_id: board.id.clone(),
            base_fingerprint: base_fingerprint.clone(),
            expected_modules,
            capability_request: args.capability_plan,
            capability_plan: capability_plan.clone(),
            request_acceptance_contract: claimed_request.contract,
            request_identity: claimed_request.request_identity,
            mode: args.mode,
            program: args.program.clone(),
            staged_evaluation: staged_evaluation.clone(),
            validated: None,
            best: None,
            committed_revision: None,
            pending_revision: None,
            pending_claim_id: None,
            pending_commands: None,
        };
        let stored_bytes = stored_draft_size(&stored);
        let mut drafts = match self.drafts.lock() {
            Ok(drafts) => drafts,
            Err(_) => {
                return FlowIrDraftResponse::error(
                    "IR_DRAFT_STORE_UNAVAILABLE",
                    "typed draft store lock is unavailable",
                );
            }
        };
        if self
            .source_drafts
            .lock()
            .is_ok_and(|source_drafts| source_drafts.contains_key(&draft_id))
        {
            return FlowIrDraftResponse::error(
                "IR_DRAFT_ID_COLLISION",
                "draft_id is already used by a retained FlowScript source draft",
            );
        }
        if let Some(existing) = drafts.get(&draft_id)
            && existing.committed_revision.is_some()
        {
            let mut response = FlowIrDraftResponse::error(
                "IR_DRAFT_COMMIT_PENDING",
                "This draft has already returned a command batch and cannot be replaced under the same id. Resolve that batch or begin a new draft id.",
            );
            response.draft_id = Some(draft_id);
            response.revision = Some(existing.revision);
            response.base_fingerprint = Some(existing.base_fingerprint.clone());
            return response;
        }
        if let Some(existing) = drafts.get(&draft_id)
            && !args.replace_existing
        {
            let mut response = FlowIrDraftResponse::error(
                "IR_DRAFT_ALREADY_EXISTS",
                "This draft id already has retained work. Continue from its current revision, or set replace_existing only when intentionally abandoning it.",
            );
            response.draft_id = Some(draft_id);
            response.revision = Some(existing.revision);
            response.base_fingerprint = Some(existing.base_fingerprint.clone());
            return response;
        }
        loop {
            let other_count = drafts.keys().filter(|id| *id != &draft_id).count();
            let other_bytes = drafts
                .iter()
                .filter(|(id, _)| *id != &draft_id)
                .map(|(_, draft)| stored_draft_size(draft))
                .sum::<usize>();
            if other_count < MAX_FLOW_IR_DRAFTS_PER_STORE
                && other_bytes.saturating_add(stored_bytes) <= MAX_FLOW_IR_DRAFT_STORE_BYTES
            {
                break;
            }
            let victim = drafts
                .iter()
                // Never discard a draft while its atomic command batch is still pending board
                // observation. Older editing or already-observed drafts remain ordinary LRU
                // candidates.
                .filter(|(id, draft)| *id != &draft_id && draft.pending_revision.is_none())
                .min_by_key(|(_, draft)| draft.access_sequence)
                .map(|(id, _)| id.clone());
            let Some(victim) = victim else {
                return FlowIrDraftResponse::error(
                    "IR_DRAFT_STORE_SIZE_LIMIT_EXCEEDED",
                    format!(
                        "typed draft store cannot exceed {MAX_FLOW_IR_DRAFT_STORE_BYTES} bytes"
                    ),
                );
            };
            drafts.remove(&victim);
        }
        drafts.insert(draft_id.clone(), stored);
        drop(drafts);
        if let Some(binding) = acceptance_binding {
            self.request_acceptance_contracts
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&binding.id);
        }
        FlowIrDraftResponse::from_staged_evaluation(
            "draft_started",
            draft_id,
            0,
            base_fingerprint,
            staged_evaluation,
            Some(capability_plan),
            missing,
            pending_capabilities,
        )
    }

    pub fn upsert_module(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        args: UpsertFlowIrModuleArgs,
    ) -> FlowIrDraftResponse {
        self.upsert_module_internal(board, catalog, args, None)
    }

    pub fn upsert_module_with_acceptance_binding(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        args: UpsertFlowIrModuleArgs,
        binding: &FlowIrAcceptanceBinding,
    ) -> FlowIrDraftResponse {
        self.upsert_module_internal(board, catalog, args, Some(binding))
    }

    fn upsert_module_internal(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        args: UpsertFlowIrModuleArgs,
        binding: Option<&FlowIrAcceptanceBinding>,
    ) -> FlowIrDraftResponse {
        let draft_key = args.draft_id.trim();
        let draft = {
            let mut drafts = match self.drafts.lock() {
                Ok(drafts) => drafts,
                Err(_) => {
                    return FlowIrDraftResponse::error(
                        "IR_DRAFT_STORE_UNAVAILABLE",
                        "typed draft store lock is unavailable",
                    );
                }
            };
            let Some(draft) = drafts.get_mut(draft_key) else {
                drop(drafts);
                self.release_missing_draft_claim(binding, draft_key);
                return FlowIrDraftResponse::error(
                    "IR_DRAFT_MISSING",
                    "begin_flow_ir_draft must be called before upserting modules",
                );
            };
            if let Some(denied) =
                draft_request_authorization_error(&board.id, draft_key, draft, binding)
            {
                return draft_request_mismatch_response(denied);
            }
            draft.access_sequence = self.next_access_sequence();
            draft.clone()
        };
        if draft.committed_revision.is_some() {
            let mut response = FlowIrDraftResponse::error(
                "IR_DRAFT_ALREADY_COMMITTED",
                "this draft has already returned an atomic command batch; begin a new draft for further changes",
            );
            response.draft_id = Some(args.draft_id);
            response.revision = Some(draft.revision);
            response.base_fingerprint = Some(draft.base_fingerprint.clone());
            return response;
        }
        if args.expected_revision != draft.revision {
            return FlowIrDraftResponse::revision_conflict(
                args.draft_id,
                draft.revision,
                args.expected_revision,
            );
        }

        let module_key = normalize(args.module.name());
        if module_key.is_empty() || args.module.name().len() > MAX_FLOW_IR_AUTHORED_NAME_BYTES {
            return FlowIrDraftResponse::error(
                "IR_MODULE_NAME_INVALID",
                format!(
                    "module name must be non-empty and at most {MAX_FLOW_IR_AUTHORED_NAME_BYTES} bytes"
                ),
            );
        }
        let existing_index = draft
            .program
            .modules
            .iter()
            .position(|module| normalize(module.name()) == module_key);
        let mut candidate = draft.program.clone();
        match existing_index {
            Some(index) => candidate.modules[index] = args.module,
            None => candidate.modules.push(args.module),
        }
        let resource_diagnostics = validate_ir_resource_limits(&candidate);
        if !resource_diagnostics.is_empty() {
            return retained_staged_draft_response(
                "resource_limit_rejected",
                "IR_RESOURCE_LIMIT_EXCEEDED",
                "The oversized module replacement was rejected; continue from the retained revision.",
                args.draft_id,
                &draft,
                resource_diagnostics,
            );
        }
        let candidate_evaluation =
            self.evaluate_staged_program(catalog, candidate.clone(), &draft.expected_modules);

        let removed_scope_items =
            removed_module_scope_items(&draft.program, &candidate, &module_key);
        if existing_index.is_some()
            && !removed_scope_items.is_empty()
            && !args.allow_scope_reduction
        {
            let diagnostic = FlowIrDiagnostic {
                code: "IR_SCOPE_REDUCTION_NOT_ALLOWED".to_string(),
                phase: "draft".to_string(),
                path: "/module/steps".to_string(),
                scope: Some(args.draft_id.clone()),
                message: format!(
                    "replacement removed retained scope from module {module_key:?}: {}; previous revision was retained",
                    removed_scope_items.join(", ")
                ),
                expected: Some(
                    "preserve every retained parameter, return, and step identity".to_string(),
                ),
                actual: Some(format!(
                    "removed {} retained item(s)",
                    removed_scope_items.len()
                )),
                declaration: Some(module_key.clone()),
                pin: None,
                fix: Some(
                    "restore the omitted workflow behavior, or set allow_scope_reduction only when the user explicitly requested a smaller scope"
                        .to_string(),
                ),
                caused_by: Vec::new(),
            };
            return FlowIrDraftResponse {
                status: "scope_reduction_blocked".to_string(),
                code: Some("IR_SCOPE_REDUCTION_NOT_ALLOWED".to_string()),
                message:
                    "The structurally smaller module replacement was rejected; continue from the retained revision."
                        .to_string(),
                draft_id: Some(args.draft_id),
                revision: Some(draft.revision),
                base_fingerprint: Some(draft.base_fingerprint.clone()),
                diagnostics: vec![diagnostic],
                module_node_counts: draft.staged_evaluation.compile.module_node_counts.clone(),
                flowscript: Some(draft.staged_evaluation.compile.flowscript.clone()),
                retained_ir: None,
                capability_plan: Some(draft.capability_plan.clone()),
                remaining_capabilities: pending_required_capability_ids(&draft.capability_request),
                missing_modules: missing_modules(&draft),
                derived_command_count: None,
                commands: Vec::new(),
            };
        }

        // A module replacement is compared only with the cached diagnostics attributable to that
        // module. Unrelated incomplete modules cannot make every small repair pay for or regress a
        // whole-program evaluation.
        let current_local_diagnostics =
            staged_module_diagnostics(&draft.staged_evaluation, &draft.program, &module_key);
        let candidate_local_diagnostics =
            staged_module_diagnostics(&candidate_evaluation, &candidate, &module_key);
        if existing_index.is_some()
            && candidate_local_diagnostics.len() > current_local_diagnostics.len()
        {
            let mut diagnostics = candidate_local_diagnostics.clone();
            diagnostics.insert(
                0,
                FlowIrDiagnostic {
                    code: "IR_CANDIDATE_REGRESSION".to_string(),
                    phase: "draft".to_string(),
                    path: "/module".to_string(),
                    scope: Some(args.draft_id.clone()),
                    message: format!(
                        "replacement increased diagnostics from {} to {}; previous revision was retained",
                        current_local_diagnostics.len(),
                        candidate_local_diagnostics.len()
                    ),
                    expected: Some(format!("<= {} diagnostics", current_local_diagnostics.len())),
                    actual: Some(format!("{} diagnostics", candidate_local_diagnostics.len())),
                    declaration: None,
                    pin: None,
                    fix: Some(
                        "repair the reported root diagnostic without removing or destabilizing the prior module"
                            .to_string(),
                    ),
                    caused_by: candidate_local_diagnostics
                        .iter()
                        .map(|diagnostic| diagnostic.code.clone())
                        .collect(),
                },
            );
            return FlowIrDraftResponse {
                status: "candidate_regression".to_string(),
                code: Some("IR_CANDIDATE_REGRESSION".to_string()),
                message: "The worsening module replacement was rejected; continue from the retained revision."
                    .to_string(),
                draft_id: Some(args.draft_id),
                revision: Some(draft.revision),
                base_fingerprint: Some(draft.base_fingerprint.clone()),
                diagnostics,
                module_node_counts: draft.staged_evaluation.compile.module_node_counts.clone(),
                flowscript: Some(draft.staged_evaluation.compile.flowscript.clone()),
                retained_ir: None,
                capability_plan: Some(draft.capability_plan.clone()),
                remaining_capabilities: pending_required_capability_ids(&draft.capability_request),
                missing_modules: missing_modules(&draft),
                derived_command_count: None,
                commands: Vec::new(),
            };
        }

        let revision = draft.revision.saturating_add(1);
        let present_modules = candidate
            .modules
            .iter()
            .map(|module| normalize(module.name()))
            .collect::<HashSet<_>>();
        let mut missing = draft
            .expected_modules
            .keys()
            .filter(|name| !present_modules.contains(*name))
            .cloned()
            .collect::<Vec<_>>();
        missing.sort();
        let mut accepted = draft.clone();
        accepted.revision = revision;
        accepted.program = candidate;
        accepted.staged_evaluation = candidate_evaluation.clone();
        accepted.validated = None;
        if args.allow_scope_reduction {
            // Do not let use_best_candidate resurrect behavior from before an explicitly
            // authorized reduction. The accepted replacement starts a new candidate lineage.
            accepted.best = None;
        }
        let state_sequence = self.next_access_sequence();
        accepted.state_sequence = state_sequence;
        accepted.access_sequence = state_sequence;
        {
            let mut drafts = match self.drafts.lock() {
                Ok(drafts) => drafts,
                Err(_) => {
                    return FlowIrDraftResponse::error(
                        "IR_DRAFT_STORE_UNAVAILABLE",
                        "typed draft store lock is unavailable",
                    );
                }
            };
            let Some(current) = drafts.get(draft_key) else {
                return FlowIrDraftResponse::error(
                    "IR_DRAFT_MISSING",
                    "typed draft disappeared while its module was being validated",
                );
            };
            if current.revision != draft.revision || current.state_sequence != draft.state_sequence
            {
                return FlowIrDraftResponse::revision_conflict(
                    args.draft_id,
                    current.revision,
                    args.expected_revision,
                );
            }
            let other_draft_bytes = drafts
                .iter()
                .filter(|(id, _)| id.as_str() != draft_key)
                .map(|(_, draft)| stored_draft_size(draft))
                .sum::<usize>();
            if other_draft_bytes.saturating_add(stored_draft_size(&accepted))
                > MAX_FLOW_IR_DRAFT_STORE_BYTES
            {
                return retained_staged_draft_response(
                    "resource_limit_rejected",
                    "IR_DRAFT_STORE_SIZE_LIMIT_EXCEEDED",
                    "The accepted module and retained best candidate would exceed the byte budget.",
                    args.draft_id,
                    &draft,
                    Vec::new(),
                );
            }
            drafts.insert(draft_key.to_string(), accepted.clone());
        }
        FlowIrDraftResponse::from_staged_evaluation_with_diagnostics(
            if candidate_local_diagnostics.is_empty() {
                "module_validated"
            } else {
                "module_needs_repair"
            },
            args.draft_id,
            revision,
            accepted.base_fingerprint,
            candidate_evaluation,
            candidate_local_diagnostics,
            Some(accepted.capability_plan),
            missing,
            pending_required_capability_ids(&accepted.capability_request),
        )
    }

    /// Repair the retained program header and remove mistakenly authored modules without forcing
    /// the model to restart the draft and replay every valid module. Header entries are complete
    /// upserts keyed by name; explicit removals remain scope-gated.
    pub fn update_draft(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        args: UpdateFlowIrDraftArgs,
    ) -> FlowIrDraftResponse {
        self.update_draft_internal(board, catalog, args, None)
    }

    pub fn update_draft_with_acceptance_binding(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        args: UpdateFlowIrDraftArgs,
        binding: &FlowIrAcceptanceBinding,
    ) -> FlowIrDraftResponse {
        self.update_draft_internal(board, catalog, args, Some(binding))
    }

    fn update_draft_internal(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        args: UpdateFlowIrDraftArgs,
        binding: Option<&FlowIrAcceptanceBinding>,
    ) -> FlowIrDraftResponse {
        let draft_key = args.draft_id.trim();
        let draft = {
            let mut drafts = match self.drafts.lock() {
                Ok(drafts) => drafts,
                Err(_) => {
                    return FlowIrDraftResponse::error(
                        "IR_DRAFT_STORE_UNAVAILABLE",
                        "typed draft store lock is unavailable",
                    );
                }
            };
            let Some(draft) = drafts.get_mut(draft_key) else {
                drop(drafts);
                self.release_missing_draft_claim(binding, draft_key);
                return FlowIrDraftResponse::error(
                    "IR_DRAFT_MISSING",
                    "begin_flow_ir_draft must be called before updating its header or module set",
                );
            };
            if let Some(denied) =
                draft_request_authorization_error(&board.id, draft_key, draft, binding)
            {
                return draft_request_mismatch_response(denied);
            }
            draft.access_sequence = self.next_access_sequence();
            draft.clone()
        };
        if draft.committed_revision.is_some() {
            let mut response = FlowIrDraftResponse::error(
                "IR_DRAFT_ALREADY_COMMITTED",
                "this draft has already returned an atomic command batch; begin a new draft for further changes",
            );
            response.draft_id = Some(args.draft_id);
            response.revision = Some(draft.revision);
            response.base_fingerprint = Some(draft.base_fingerprint.clone());
            return response;
        }
        if args.expected_revision != draft.revision {
            return FlowIrDraftResponse::revision_conflict(
                args.draft_id,
                draft.revision,
                args.expected_revision,
            );
        }
        if args.interfaces.is_empty()
            && args.variables.is_empty()
            && args.remove_modules.is_empty()
            && args.remove_interfaces.is_empty()
            && args.remove_variables.is_empty()
            && args.expected_modules.is_none()
            && args.capability_plan.is_none()
        {
            let mut response = FlowIrDraftResponse::error(
                "IR_DRAFT_UPDATE_EMPTY",
                "supply a retained contract replacement, interface/variable upsert, or explicit removal",
            );
            response.draft_id = Some(args.draft_id);
            response.revision = Some(draft.revision);
            response.base_fingerprint = Some(draft.base_fingerprint.clone());
            return response;
        }
        if args
            .remove_modules
            .len()
            .saturating_add(args.remove_interfaces.len())
            .saturating_add(args.remove_variables.len())
            > MAX_FLOW_IR_ENTITY_ALLOWLIST_ITEMS
        {
            return FlowIrDraftResponse::error(
                "IR_DRAFT_REMOVAL_LIMIT_EXCEEDED",
                format!(
                    "a draft update may remove at most {MAX_FLOW_IR_ENTITY_ALLOWLIST_ITEMS} retained items"
                ),
            );
        }
        if args
            .interfaces
            .iter()
            .map(|interface| interface.name.as_str())
            .chain(args.variables.iter().map(|variable| variable.name.as_str()))
            .chain(args.remove_modules.iter().map(String::as_str))
            .chain(args.remove_interfaces.iter().map(String::as_str))
            .chain(args.remove_variables.iter().map(String::as_str))
            .chain(args.expected_modules.iter().flatten().map(String::as_str))
            .any(|name| name.len() > MAX_FLOW_IR_AUTHORED_NAME_BYTES)
        {
            return FlowIrDraftResponse::error(
                "IR_AUTHORED_NAME_TOO_LONG",
                format!(
                    "draft update names must be at most {MAX_FLOW_IR_AUTHORED_NAME_BYTES} bytes"
                ),
            );
        }
        if let Some(expected_modules) = &args.expected_modules {
            if expected_modules.len() > super::ir::MAX_FLOW_IR_MODULES {
                return FlowIrDraftResponse::error(
                    "IR_EXPECTED_MODULE_LIMIT_EXCEEDED",
                    format!(
                        "expected_modules may contain at most {} entries",
                        super::ir::MAX_FLOW_IR_MODULES
                    ),
                );
            }
            if normalized_name_set(expected_modules).is_empty() {
                return FlowIrDraftResponse::error(
                    "IR_EXPECTED_MODULES_REQUIRED",
                    "expected_modules must retain at least one required function or event",
                );
            }
        }
        if let Some(capability_request) = &args.capability_plan
            && let Some((code, message)) = capability_request_limit_error(capability_request)
        {
            return FlowIrDraftResponse::error(code, message);
        }

        let current_evaluation = draft.staged_evaluation.clone();
        let mut candidate = draft.program.clone();
        let candidate_capability_request = args
            .capability_plan
            .clone()
            .unwrap_or_else(|| draft.capability_request.clone());
        let candidate_expected_names = args
            .expected_modules
            .clone()
            .unwrap_or_else(|| draft.expected_modules.keys().cloned().collect());
        let mut candidate_expected_modules = match expected_module_contract(
            &candidate_expected_names,
            &candidate_capability_request,
        ) {
            Ok(expected) => expected,
            Err((code, message)) => return FlowIrDraftResponse::error(code, message),
        };
        let mut reduced_items = Vec::new();
        for (name, kind) in &draft.expected_modules {
            match candidate_expected_modules.get(name) {
                Some(candidate_kind) if candidate_kind == kind => {}
                Some(candidate_kind) => reduced_items.push(format!(
                    "required module {name} kind {kind:?} -> {candidate_kind:?}"
                )),
                None => reduced_items.push(format!("required module {name}")),
            }
        }
        reduced_items.extend(
            removed_required_capabilities(&draft.capability_request, &candidate_capability_request)
                .into_iter()
                .map(|id| format!("required capability {id}")),
        );

        for interface in args.interfaces {
            let key = normalize(&interface.name);
            if key.is_empty() {
                return FlowIrDraftResponse::error(
                    "IR_INTERFACE_NAME_INVALID",
                    "interface upsert names must be non-empty",
                );
            }
            if let Some(index) = candidate
                .interfaces
                .iter()
                .position(|existing| normalize(&existing.name) == key)
            {
                let replacement_fields = interface
                    .fields
                    .iter()
                    .map(|field| normalize(&field.name))
                    .collect::<HashSet<_>>();
                let mut removed_fields = candidate.interfaces[index]
                    .fields
                    .iter()
                    .map(|field| normalize(&field.name))
                    .filter(|field| !replacement_fields.contains(field))
                    .collect::<Vec<_>>();
                removed_fields.sort();
                if !removed_fields.is_empty() {
                    reduced_items.push(format!(
                        "interface {key} fields [{}]",
                        removed_fields.join(", ")
                    ));
                }
                candidate.interfaces[index] = interface;
            } else {
                candidate.interfaces.push(interface);
            }
        }
        for variable in args.variables {
            let key = normalize(&variable.name);
            if key.is_empty() {
                return FlowIrDraftResponse::error(
                    "IR_VARIABLE_NAME_INVALID",
                    "variable upsert names must be non-empty",
                );
            }
            if let Some(index) = candidate
                .variables
                .iter()
                .position(|existing| normalize(&existing.name) == key)
            {
                candidate.variables[index] = variable;
            } else {
                candidate.variables.push(variable);
            }
        }

        let remove_modules = normalized_name_set(&args.remove_modules);
        let remove_interfaces = normalized_name_set(&args.remove_interfaces);
        let remove_variables = normalized_name_set(&args.remove_variables);
        for name in &remove_modules {
            if draft.expected_modules.contains_key(name) {
                reduced_items.push(format!("required module {name}"));
            }
        }
        candidate_expected_modules.retain(|name, _| !remove_modules.contains(name));
        reduced_items.extend(
            remove_interfaces
                .iter()
                .map(|name| format!("interface {name}")),
        );
        reduced_items.extend(
            remove_variables
                .iter()
                .map(|name| format!("variable {name}")),
        );
        reduced_items.sort();
        reduced_items.dedup();
        if !reduced_items.is_empty() && !args.allow_scope_reduction {
            let diagnostic = FlowIrDiagnostic {
                code: "IR_SCOPE_REDUCTION_NOT_ALLOWED".to_string(),
                phase: "draft".to_string(),
                path: "/".to_string(),
                scope: Some(args.draft_id.clone()),
                message: format!(
                    "draft update would reduce {}; previous revision was retained",
                    reduced_items.join(", ")
                ),
                expected: Some("preserve the retained program scope".to_string()),
                actual: Some(format!("remove/reduce {} item(s)", reduced_items.len())),
                declaration: None,
                pin: None,
                fix: Some(
                    "upsert a complete replacement, or allow scope reduction only when the user explicitly requested removal"
                        .to_string(),
                ),
                caused_by: Vec::new(),
            };
            return retained_draft_response(
                "scope_reduction_blocked",
                "IR_SCOPE_REDUCTION_NOT_ALLOWED",
                "The smaller draft update was rejected; continue from the retained revision.",
                args.draft_id,
                &draft,
                current_evaluation,
                vec![diagnostic],
            );
        }

        candidate
            .modules
            .retain(|module| !remove_modules.contains(&normalize(module.name())));
        candidate
            .interfaces
            .retain(|interface| !remove_interfaces.contains(&normalize(&interface.name)));
        candidate
            .variables
            .retain(|variable| !remove_variables.contains(&normalize(&variable.name)));
        if candidate_expected_modules.is_empty() {
            return retained_draft_response(
                "scope_reduction_blocked",
                "IR_EXPECTED_MODULES_REQUIRED",
                "A typed draft must retain at least one expected module.",
                args.draft_id,
                &draft,
                current_evaluation,
                Vec::new(),
            );
        }
        if candidate == draft.program
            && candidate_expected_modules == draft.expected_modules
            && candidate_capability_request == draft.capability_request
        {
            let mut response = FlowIrDraftResponse::error(
                "IR_DRAFT_UPDATE_NO_CHANGES",
                "the requested draft update did not change any retained header entry or module",
            );
            response.draft_id = Some(args.draft_id);
            response.revision = Some(draft.revision);
            response.base_fingerprint = Some(draft.base_fingerprint.clone());
            return response;
        }

        let resource_diagnostics = validate_ir_resource_limits(&candidate);
        if !resource_diagnostics.is_empty() {
            return retained_draft_response(
                "resource_limit_rejected",
                "IR_RESOURCE_LIMIT_EXCEEDED",
                "The oversized draft update was rejected; continue from the retained revision.",
                args.draft_id,
                &draft,
                current_evaluation,
                resource_diagnostics,
            );
        }
        let candidate_capability_plan = if candidate_capability_request == draft.capability_request
        {
            draft.capability_plan.clone()
        } else {
            plan_flow_capabilities(&candidate_capability_request, catalog)
        };
        let candidate_evaluation =
            self.evaluate_staged_program(catalog, candidate.clone(), &candidate_expected_modules);
        if candidate_evaluation.diagnostic_count() > current_evaluation.diagnostic_count() {
            let mut diagnostics = candidate_evaluation.diagnostics.clone();
            diagnostics.insert(
                0,
                FlowIrDiagnostic {
                    code: "IR_CANDIDATE_REGRESSION".to_string(),
                    phase: "draft".to_string(),
                    path: "/".to_string(),
                    scope: Some(args.draft_id.clone()),
                    message: format!(
                        "draft update increased diagnostics from {} to {}; previous revision was retained",
                        current_evaluation.diagnostic_count(),
                        candidate_evaluation.diagnostic_count()
                    ),
                    expected: Some(format!(
                        "<= {} diagnostics",
                        current_evaluation.diagnostic_count()
                    )),
                    actual: Some(format!(
                        "{} diagnostics",
                        candidate_evaluation.diagnostic_count()
                    )),
                    declaration: None,
                    pin: None,
                    fix: Some(
                        "repair the reported root header/module diagnostic without destabilizing retained work"
                            .to_string(),
                    ),
                    caused_by: candidate_evaluation
                        .diagnostics
                        .iter()
                        .map(|diagnostic| diagnostic.code.clone())
                        .collect(),
                },
            );
            return retained_draft_response(
                "candidate_regression",
                "IR_CANDIDATE_REGRESSION",
                "The worsening draft update was rejected; continue from the retained revision.",
                args.draft_id,
                &draft,
                current_evaluation,
                diagnostics,
            );
        }

        let revision = draft.revision.saturating_add(1);
        let present_modules = candidate
            .modules
            .iter()
            .map(|module| normalize(module.name()))
            .collect::<HashSet<_>>();
        let mut missing = candidate_expected_modules
            .keys()
            .filter(|name| !present_modules.contains(*name))
            .cloned()
            .collect::<Vec<_>>();
        missing.sort();

        let mut accepted = draft.clone();
        accepted.revision = revision;
        accepted.program = candidate;
        accepted.expected_modules = candidate_expected_modules;
        accepted.capability_request = candidate_capability_request;
        accepted.capability_plan = candidate_capability_plan.clone();
        accepted.staged_evaluation = candidate_evaluation.clone();
        accepted.validated = None;
        if args.allow_scope_reduction {
            // Header/module removal changes user intent. A previously valid best candidate belongs
            // to the old intent and cannot remain eligible for commit.
            accepted.best = None;
        }
        let state_sequence = self.next_access_sequence();
        accepted.state_sequence = state_sequence;
        accepted.access_sequence = state_sequence;
        {
            let mut drafts = match self.drafts.lock() {
                Ok(drafts) => drafts,
                Err(_) => {
                    return FlowIrDraftResponse::error(
                        "IR_DRAFT_STORE_UNAVAILABLE",
                        "typed draft store lock is unavailable",
                    );
                }
            };
            let Some(current) = drafts.get(draft_key) else {
                return FlowIrDraftResponse::error(
                    "IR_DRAFT_MISSING",
                    "typed draft disappeared while its header was being validated",
                );
            };
            if current.revision != draft.revision || current.state_sequence != draft.state_sequence
            {
                return FlowIrDraftResponse::revision_conflict(
                    args.draft_id,
                    current.revision,
                    args.expected_revision,
                );
            }
            let other_draft_bytes = drafts
                .iter()
                .filter(|(id, _)| id.as_str() != draft_key)
                .map(|(_, draft)| stored_draft_size(draft))
                .sum::<usize>();
            if other_draft_bytes.saturating_add(stored_draft_size(&accepted))
                > MAX_FLOW_IR_DRAFT_STORE_BYTES
            {
                return retained_staged_draft_response(
                    "resource_limit_rejected",
                    "IR_DRAFT_STORE_SIZE_LIMIT_EXCEEDED",
                    "The accepted draft update and retained best candidate would exceed the byte budget.",
                    args.draft_id,
                    &draft,
                    Vec::new(),
                );
            }
            drafts.insert(draft_key.to_string(), accepted.clone());
        }
        FlowIrDraftResponse::from_staged_evaluation(
            if candidate_evaluation.diagnostics.is_empty() {
                "draft_updated"
            } else {
                "draft_needs_repair"
            },
            args.draft_id,
            revision,
            accepted.base_fingerprint,
            candidate_evaluation,
            Some(candidate_capability_plan),
            missing,
            pending_required_capability_ids(&accepted.capability_request),
        )
    }

    pub fn validate(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        args: ValidateFlowIrDraftArgs,
    ) -> FlowIrDraftResponse {
        self.validate_with_optional_binding(board, catalog, args, None)
    }

    pub fn validate_with_acceptance_binding(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        args: ValidateFlowIrDraftArgs,
        binding: &FlowIrAcceptanceBinding,
    ) -> FlowIrDraftResponse {
        self.validate_with_optional_binding(board, catalog, args, Some(binding))
    }

    fn validate_with_optional_binding(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        args: ValidateFlowIrDraftArgs,
        binding: Option<&FlowIrAcceptanceBinding>,
    ) -> FlowIrDraftResponse {
        let draft_key = args.draft_id.trim();
        let draft = {
            let mut drafts = match self.drafts.lock() {
                Ok(drafts) => drafts,
                Err(_) => {
                    return FlowIrDraftResponse::error(
                        "IR_DRAFT_STORE_UNAVAILABLE",
                        "typed draft store lock is unavailable",
                    );
                }
            };
            let Some(draft) = drafts.get_mut(draft_key) else {
                drop(drafts);
                self.release_missing_draft_claim(binding, draft_key);
                return FlowIrDraftResponse::error(
                    "IR_DRAFT_MISSING",
                    "typed draft does not exist",
                );
            };
            if let Some(denied) =
                draft_request_authorization_error(&board.id, draft_key, draft, binding)
            {
                return draft_request_mismatch_response(denied);
            }
            draft.access_sequence = self.next_access_sequence();
            draft.clone()
        };
        if args.modules.len() > super::ir::MAX_FLOW_IR_MODULES {
            return FlowIrDraftResponse::error(
                "IR_RETAINED_MODULE_LIMIT_EXCEEDED",
                format!(
                    "select at most {} retained modules per validation response",
                    super::ir::MAX_FLOW_IR_MODULES
                ),
            );
        }
        let requested_modules = normalized_name_set(&args.modules);
        let available_modules = draft
            .program
            .modules
            .iter()
            .map(|module| normalize(module.name()))
            .collect::<HashSet<_>>();
        let mut unknown_modules = requested_modules
            .difference(&available_modules)
            .cloned()
            .collect::<Vec<_>>();
        unknown_modules.sort();
        if !unknown_modules.is_empty() {
            let mut response = FlowIrDraftResponse::error(
                "IR_RETAINED_MODULE_MISSING",
                format!(
                    "requested retained modules do not exist: {}",
                    unknown_modules.join(", ")
                ),
            );
            response.draft_id = Some(args.draft_id);
            response.revision = Some(draft.revision);
            response.base_fingerprint = Some(draft.base_fingerprint.clone());
            return response;
        }
        let retained_ir =
            (args.include_header || !requested_modules.is_empty()).then(|| FlowIrProgram {
                version: draft.program.version.clone(),
                interfaces: if args.include_header {
                    draft.program.interfaces.clone()
                } else {
                    Vec::new()
                },
                variables: if args.include_header {
                    draft.program.variables.clone()
                } else {
                    Vec::new()
                },
                modules: draft
                    .program
                    .modules
                    .iter()
                    .filter(|module| requested_modules.contains(&normalize(module.name())))
                    .cloned()
                    .collect(),
            });
        let evaluation = self
            .evaluate_complete_program(
                board,
                catalog,
                draft.program.clone(),
                &draft.capability_request,
                &draft.capability_plan,
                &draft.expected_modules,
                draft.mode,
            )
            .complete(acceptance_contract_diagnostics(
                &draft.request_acceptance_contract,
                &draft.program,
                catalog,
            ));
        let plan = draft.capability_plan.clone();
        let missing = missing_modules(&draft);
        let status = if evaluation.diagnostics.is_empty() && missing.is_empty() && plan.feasible {
            "draft_valid"
        } else {
            "draft_needs_repair"
        };
        let exact_remaining = remaining_capability_ids(&evaluation.diagnostics, Some(&plan));
        let valid_best = evaluation.clone().is_valid() && missing.is_empty() && plan.feasible;
        {
            let mut drafts = match self.drafts.lock() {
                Ok(drafts) => drafts,
                Err(_) => {
                    return FlowIrDraftResponse::error(
                        "IR_DRAFT_STORE_UNAVAILABLE",
                        "typed draft store lock is unavailable",
                    );
                }
            };
            let Some(current) = drafts.get(draft_key) else {
                return FlowIrDraftResponse::error(
                    "IR_DRAFT_MISSING",
                    "typed draft disappeared while full validation was running",
                );
            };
            if current.revision != draft.revision || current.state_sequence != draft.state_sequence
            {
                return FlowIrDraftResponse::revision_conflict(
                    args.draft_id,
                    current.revision,
                    draft.revision,
                );
            }
            let mut validated = current.clone();
            let state_sequence = self.next_access_sequence();
            validated.state_sequence = state_sequence;
            validated.access_sequence = state_sequence;
            validated.validated = Some(CachedValidationSummary {
                revision: draft.revision,
                diagnostics: evaluation.diagnostics.clone(),
                remaining_capabilities: exact_remaining,
                valid: valid_best,
            });
            if valid_best {
                validated.best = Some((draft.revision, draft.program.clone()));
            }
            // A validation cache is an optimization, never a reason to evict another draft or make
            // an otherwise valid revision uncommittable. If duplicating the best candidate would
            // exceed the retained budget, commit safely falls back to the current program.
            let other_draft_bytes = drafts
                .iter()
                .filter(|(id, _)| id.as_str() != draft_key)
                .map(|(_, draft)| stored_draft_size(draft))
                .sum::<usize>();
            if other_draft_bytes.saturating_add(stored_draft_size(&validated))
                > MAX_FLOW_IR_DRAFT_STORE_BYTES
            {
                validated.best = current.best.clone();
            }
            drafts.insert(draft_key.to_string(), validated);
        }
        let mut response = FlowIrDraftResponse::from_evaluation(
            status,
            args.draft_id,
            draft.revision,
            draft.base_fingerprint.clone(),
            evaluation,
            Some(plan),
            missing,
        );
        response.retained_ir = retained_ir;
        response
    }

    pub fn commit(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        args: CommitFlowIrDraftArgs,
    ) -> FlowIrCommitResult {
        self.commit_with_optional_binding(board, catalog, args, None)
    }

    pub fn commit_with_acceptance_binding(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        args: CommitFlowIrDraftArgs,
        binding: &FlowIrAcceptanceBinding,
    ) -> FlowIrCommitResult {
        self.commit_with_optional_binding(board, catalog, args, Some(binding))
    }

    fn commit_with_optional_binding(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        args: CommitFlowIrDraftArgs,
        binding: Option<&FlowIrAcceptanceBinding>,
    ) -> FlowIrCommitResult {
        let removal_ids = args
            .remove_node_ids
            .iter()
            .chain(&args.remove_variable_ids)
            .chain(&args.remove_layer_ids)
            .chain(&args.remove_comment_ids);
        if removal_ids.clone().count() > MAX_FLOW_IR_ENTITY_ALLOWLIST_ITEMS {
            return FlowIrCommitResult::error(
                "IR_DELETION_ALLOWLIST_LIMIT_EXCEEDED",
                format!(
                    "a commit may authorize at most {MAX_FLOW_IR_ENTITY_ALLOWLIST_ITEMS} entity removals"
                ),
            );
        }
        if removal_ids
            .clone()
            .any(|id| id.len() > MAX_FLOW_IR_AUTHORED_NAME_BYTES)
        {
            return FlowIrCommitResult::error(
                "IR_DELETION_ID_TOO_LONG",
                format!("deletion ids must be at most {MAX_FLOW_IR_AUTHORED_NAME_BYTES} bytes"),
            );
        }
        let draft_key = args.draft_id.trim();
        let draft = {
            let mut drafts = match self.drafts.lock() {
                Ok(drafts) => drafts,
                Err(_) => {
                    return FlowIrCommitResult::error(
                        "IR_DRAFT_STORE_UNAVAILABLE",
                        "typed draft store lock is unavailable",
                    );
                }
            };
            let Some(target) = drafts.get(draft_key) else {
                drop(drafts);
                self.release_missing_draft_claim(binding, draft_key);
                return FlowIrCommitResult::error("IR_DRAFT_MISSING", "typed draft does not exist");
            };
            if let Some(denied) =
                draft_request_authorization_error(&board.id, draft_key, target, binding)
            {
                return FlowIrCommitResult::error(denied.code, denied.message);
            }
            let target_base_fingerprint = target.base_fingerprint.clone();
            let source_pending = self.source_drafts.lock().is_ok_and(|source_drafts| {
                source_drafts.values().any(|draft| {
                    draft.pending_revision.is_some()
                        && draft.base_fingerprint == target_base_fingerprint
                })
            });
            if source_pending
                || drafts.iter().any(|(id, draft)| {
                    id != draft_key
                        && draft.pending_revision.is_some()
                        && draft.base_fingerprint == target_base_fingerprint
                })
            {
                return FlowIrCommitResult::error(
                    "IR_BOARD_COMMIT_PENDING",
                    "another draft derived from this board revision already returned a command batch; apply or resolve it before committing a competing draft",
                );
            }
            let target = drafts
                .get_mut(draft_key)
                .expect("draft existence was checked while holding the store lock");
            target.access_sequence = self.next_access_sequence();
            target.clone()
        };
        if args.expected_revision != draft.revision {
            return FlowIrCommitResult::error(
                "IR_REVISION_CONFLICT",
                format!(
                    "commit expected revision {}, but current revision is {}",
                    args.expected_revision, draft.revision
                ),
            );
        }
        if draft.committed_revision == Some(draft.revision) {
            return FlowIrCommitResult {
                status: "already_queued".to_string(),
                code: Some("IR_DRAFT_ALREADY_COMMITTED".to_string()),
                message: "This exact draft revision already returned its atomic command batch; the retry was accepted without queueing duplicate commands."
                    .to_string(),
                draft_id: Some(args.draft_id),
                revision: Some(draft.revision),
                selected_revision: Some(draft.revision),
                base_fingerprint: Some(draft.base_fingerprint.clone()),
                claim_id: None,
                flowscript: None,
                diagnostics: Vec::new(),
                queued_count: 0,
                commands: Vec::new(),
            };
        }
        let current_fingerprint = board_fingerprint(board);
        if current_fingerprint != draft.base_fingerprint {
            self.reopen_typed_request_acceptance_contract(binding, &draft);
            return FlowIrCommitResult::error(
                "IR_BASE_REVISION_CONFLICT",
                "the board changed after this draft began; start a new draft from the fresh board",
            );
        }
        let plan = draft.capability_plan.clone();
        if !plan.feasible {
            return FlowIrCommitResult {
                status: "infeasible".to_string(),
                code: Some("IR_CAPABILITY_PLAN_INFEASIBLE".to_string()),
                message: "Required catalog capabilities or module budgets are unavailable; no commands were queued."
                    .to_string(),
                draft_id: Some(args.draft_id),
                revision: Some(draft.revision),
                selected_revision: None,
                base_fingerprint: Some(draft.base_fingerprint.clone()),
                claim_id: None,
                flowscript: None,
                diagnostics: plan.module_budget_violations,
                queued_count: 0,
                commands: Vec::new(),
            };
        }
        let (selected_revision, selected) = if args.use_best_candidate {
            draft
                .best
                .as_ref()
                .map(|(revision, program)| (*revision, program.clone()))
                .unwrap_or_else(|| (draft.revision, draft.program.clone()))
        } else {
            (draft.revision, draft.program.clone())
        };
        // Coverage belongs to the selected program, not the mutable current revision. This makes
        // use_best_candidate safe even if an older candidate predates a required module.
        let missing = missing_modules_for_program(&draft.expected_modules, &selected);
        if !missing.is_empty() {
            return FlowIrCommitResult::error(
                "IR_REQUIRED_MODULES_MISSING",
                format!(
                    "selected candidate is missing required modules: {}",
                    missing.join(", ")
                ),
            );
        }
        let acceptance_diagnostics =
            acceptance_contract_diagnostics(&draft.request_acceptance_contract, &selected, catalog);
        let evaluation = self
            .evaluate_complete_program(
                board,
                catalog,
                selected,
                &draft.capability_request,
                &draft.capability_plan,
                &draft.expected_modules,
                draft.mode,
            )
            .complete(acceptance_diagnostics);
        if !evaluation.diagnostics.is_empty() {
            return FlowIrCommitResult {
                status: "validation_errors".to_string(),
                code: Some("IR_DRAFT_INVALID".to_string()),
                message: "Typed draft validation failed; no commands were queued.".to_string(),
                draft_id: Some(args.draft_id),
                revision: Some(draft.revision),
                selected_revision: Some(selected_revision),
                base_fingerprint: Some(draft.base_fingerprint.clone()),
                claim_id: None,
                flowscript: Some(evaluation.compile.flowscript),
                diagnostics: evaluation.diagnostics,
                queued_count: 0,
                commands: Vec::new(),
            };
        }
        let reconcile = evaluation
            .reconcile
            .expect("a diagnostic-free typed evaluation always reconciles");
        if reconcile.commands.is_empty() {
            return FlowIrCommitResult::error(
                "IR_NO_CHANGES",
                "typed draft is valid but derives no board changes",
            );
        }
        let destructive = destructive_flowscript_command_summaries(&reconcile.commands);
        let requested_node_removals = args
            .remove_node_ids
            .iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect::<HashSet<_>>();
        let requested_variable_removals = args
            .remove_variable_ids
            .iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect::<HashSet<_>>();
        let requested_layer_removals = args
            .remove_layer_ids
            .iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect::<HashSet<_>>();
        let requested_comment_removals = args
            .remove_comment_ids
            .iter()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect::<HashSet<_>>();
        if draft.mode == FlowIrDraftMode::Additive
            && (args.allow_deletions
                || !requested_node_removals.is_empty()
                || !requested_variable_removals.is_empty()
                || !requested_layer_removals.is_empty()
                || !requested_comment_removals.is_empty())
        {
            return FlowIrCommitResult::error(
                "IR_ADDITIVE_DELETION_INVALID",
                "additive drafts preserve unrelated existing board state and cannot request deletions; begin an explicit replace draft only for a user-authorized full replacement",
            );
        }

        let actual_node_removals = reconcile
            .commands
            .iter()
            .filter_map(|command| match command {
                BoardCommand::RemoveNode { node_id, .. } => Some(node_id.clone()),
                _ => None,
            })
            .collect::<HashSet<_>>();
        let actual_variable_removals = reconcile
            .commands
            .iter()
            .filter_map(|command| match command {
                BoardCommand::RemoveVariable { variable_id, .. } => Some(variable_id.clone()),
                _ => None,
            })
            .collect::<HashSet<_>>();
        let actual_layer_removals = reconcile
            .commands
            .iter()
            .filter_map(|command| match command {
                BoardCommand::RemoveLayer { layer_id, .. } => Some(layer_id.clone()),
                _ => None,
            })
            .collect::<HashSet<_>>();
        let actual_comment_removals = reconcile
            .commands
            .iter()
            .filter_map(|command| match command {
                BoardCommand::RemoveComment { comment_id, .. } => Some(comment_id.clone()),
                _ => None,
            })
            .collect::<HashSet<_>>();
        if draft.mode == FlowIrDraftMode::Replace
            && (requested_node_removals != actual_node_removals
                || requested_variable_removals != actual_variable_removals
                || requested_layer_removals != actual_layer_removals
                || requested_comment_removals != actual_comment_removals)
        {
            let mut actual_nodes = actual_node_removals.into_iter().collect::<Vec<_>>();
            let mut actual_variables = actual_variable_removals.into_iter().collect::<Vec<_>>();
            let mut actual_layers = actual_layer_removals.into_iter().collect::<Vec<_>>();
            let mut actual_comments = actual_comment_removals.into_iter().collect::<Vec<_>>();
            actual_nodes.sort();
            actual_variables.sort();
            actual_layers.sort();
            actual_comments.sort();
            return FlowIrCommitResult::error(
                "IR_DELETION_ALLOWLIST_MISMATCH",
                format!(
                    "replacement derived exact node removals {actual_nodes:?}, variable removals {actual_variables:?}, layer removals {actual_layers:?}, and comment removals {actual_comments:?}; commit must enumerate those ids exactly (a global allow_deletions flag is insufficient)"
                ),
            );
        }
        if !destructive.is_empty() && draft.mode != FlowIrDraftMode::Replace {
            return FlowIrCommitResult::error(
                "IR_DELETION_NOT_ALLOWED",
                format!(
                    "typed additive draft unexpectedly derived deletions: {}",
                    destructive.join(", ")
                ),
            );
        }
        let commands = reconcile.commands;
        let queued_count = commands.len();
        let claim_id = create_id();
        let mut prospective = draft.clone();
        prospective.committed_revision = Some(draft.revision);
        prospective.pending_revision = Some(draft.revision);
        prospective.pending_claim_id = Some(claim_id.clone());
        prospective.pending_commands = Some(commands.clone());
        let state_sequence = self.next_access_sequence();
        prospective.state_sequence = state_sequence;
        prospective.access_sequence = state_sequence;
        {
            let mut drafts = match self.drafts.lock() {
                Ok(drafts) => drafts,
                Err(_) => {
                    return FlowIrCommitResult::error(
                        "IR_DRAFT_STORE_UNAVAILABLE",
                        "typed draft store lock is unavailable",
                    );
                }
            };
            let Some(current) = drafts.get(draft_key) else {
                return FlowIrCommitResult::error(
                    "IR_DRAFT_MISSING",
                    "typed draft disappeared while commit validation was running",
                );
            };
            if current.revision != draft.revision || current.state_sequence != draft.state_sequence
            {
                return FlowIrCommitResult::error(
                    "IR_REVISION_CONFLICT",
                    format!(
                        "commit validation raced with draft revision {}; current revision is {}",
                        draft.revision, current.revision
                    ),
                );
            }
            let source_pending = self.source_drafts.lock().is_ok_and(|source_drafts| {
                source_drafts.values().any(|candidate| {
                    candidate.pending_revision.is_some()
                        && candidate.base_fingerprint == draft.base_fingerprint
                })
            });
            if source_pending
                || drafts.iter().any(|(id, candidate)| {
                    id != draft_key
                        && candidate.pending_revision.is_some()
                        && candidate.base_fingerprint == draft.base_fingerprint
                })
            {
                return FlowIrCommitResult::error(
                    "IR_BOARD_COMMIT_PENDING",
                    "another draft derived from this board revision claimed a command batch while validation was running",
                );
            }
            let other_draft_bytes = drafts
                .iter()
                .filter(|(id, _)| id.as_str() != draft_key)
                .map(|(_, draft)| stored_draft_size(draft))
                .sum::<usize>();
            if other_draft_bytes.saturating_add(stored_draft_size(&prospective))
                > MAX_FLOW_IR_DRAFT_STORE_BYTES
            {
                return FlowIrCommitResult::error(
                    "IR_DRAFT_STORE_SIZE_LIMIT_EXCEEDED",
                    "The exact atomic command batch would exceed the retained-draft byte budget; no commit claim was created.",
                );
            }
            drafts.insert(draft_key.to_string(), prospective);
        }
        FlowIrCommitResult {
            status: "queued".to_string(),
            code: None,
            message: format!(
                "Typed IR compiled and reconciled {queued_count} atomic board change(s)."
            ),
            draft_id: Some(args.draft_id),
            revision: Some(draft.revision),
            selected_revision: Some(selected_revision),
            base_fingerprint: Some(draft.base_fingerprint.clone()),
            claim_id: Some(claim_id),
            flowscript: Some(evaluation.compile.flowscript),
            diagnostics: Vec::new(),
            queued_count,
            commands,
        }
    }

    /// Emergency rollback for a host that cannot construct the claim token before exposing any
    /// commands. Once a token can escape, use `release_commit_if_matches`; revision-only release is
    /// intentionally insufficient to distinguish a later retry of the same revision.
    pub fn release_commit(&self, draft_id: &str, revision: u64) -> bool {
        if let Ok(mut drafts) = self.drafts.lock()
            && let Some(draft) = drafts.get_mut(draft_id.trim())
        {
            if draft.revision != revision || draft.committed_revision != Some(revision) {
                return false;
            }
            draft.committed_revision = None;
            draft.pending_revision = None;
            draft.pending_claim_id = None;
            draft.pending_commands = None;
            let state_sequence = self.next_access_sequence();
            draft.state_sequence = state_sequence;
            draft.access_sequence = state_sequence;
            return true;
        }
        let Ok(mut drafts) = self.source_drafts.lock() else {
            return false;
        };
        let Some(draft) = drafts.get_mut(draft_id.trim()) else {
            return false;
        };
        if draft.revision != revision || draft.committed_revision != Some(revision) {
            return false;
        }
        draft.committed_revision = None;
        draft.pending_revision = None;
        draft.pending_claim_id = None;
        draft.pending_commands = None;
        let state_sequence = self.next_access_sequence();
        draft.state_sequence = state_sequence;
        draft.access_sequence = state_sequence;
        true
    }

    /// Count pending batches whose base differs from the live board. Observation alone never
    /// resolves them: a changed board may be an unrelated edit while an old review is still open.
    /// Hosts must explicitly acknowledge Apply or Dismiss through the revision-scoped methods.
    pub fn observe_board(&self, board: &Board) -> usize {
        let current_fingerprint = board_fingerprint(board);
        let typed = self
            .drafts
            .lock()
            .ok()
            .map(|drafts| {
                drafts
                    .values()
                    .filter(|draft| {
                        draft.pending_revision.is_some()
                            && draft.base_fingerprint != current_fingerprint
                    })
                    .count()
            })
            .unwrap_or_default();
        let source = self
            .source_drafts
            .lock()
            .ok()
            .map(|drafts| {
                drafts
                    .values()
                    .filter(|draft| {
                        draft.pending_revision.is_some()
                            && draft.base_fingerprint != current_fingerprint
                    })
                    .count()
            })
            .unwrap_or_default();
        typed.saturating_add(source)
    }

    /// Verify that a disposition token still names the exact pending batch. Unlike Apply
    /// preflight, this intentionally does not require the live board to remain at the base
    /// fingerprint so a stale review can still be dismissed safely.
    pub fn pending_commit_matches(
        &self,
        draft_id: &str,
        revision: u64,
        base_fingerprint: &str,
        claim_id: &str,
    ) -> bool {
        let typed_match = self.drafts.lock().is_ok_and(|drafts| {
            drafts.get(draft_id.trim()).is_some_and(|draft| {
                draft.revision == revision
                    && draft.committed_revision == Some(revision)
                    && draft.pending_revision == Some(revision)
                    && draft.base_fingerprint == base_fingerprint
                    && draft.pending_claim_id.as_deref() == Some(claim_id)
                    && draft.pending_commands.is_some()
            })
        });
        typed_match
            || self.source_drafts.lock().is_ok_and(|drafts| {
                drafts.get(draft_id.trim()).is_some_and(|draft| {
                    draft.revision == revision
                        && draft.committed_revision == Some(revision)
                        && draft.pending_revision == Some(revision)
                        && draft.base_fingerprint == base_fingerprint
                        && draft.pending_claim_id.as_deref() == Some(claim_id)
                        && draft.pending_commands.is_some()
                })
            })
    }

    /// Return the host-side review policy for an exact pending batch. Replacement mode is always
    /// destructive-review gated, even when the current board happens to be empty and reconcile
    /// derives no removal commands. Callers must not treat the serialized token flag as authority.
    pub fn pending_commit_requires_destructive_approval(
        &self,
        draft_id: &str,
        revision: u64,
        base_fingerprint: &str,
        claim_id: &str,
    ) -> Option<bool> {
        let typed = self.drafts.lock().ok().and_then(|drafts| {
            let draft = drafts.get(draft_id.trim())?;
            (draft.revision == revision
                && draft.committed_revision == Some(revision)
                && draft.pending_revision == Some(revision)
                && draft.base_fingerprint == base_fingerprint
                && draft.pending_claim_id.as_deref() == Some(claim_id)
                && draft.pending_commands.is_some())
            .then_some(draft.mode == FlowIrDraftMode::Replace)
        });
        typed.or_else(|| {
            self.source_drafts.lock().ok().and_then(|drafts| {
                let draft = drafts.get(draft_id.trim())?;
                (draft.revision == revision
                    && draft.committed_revision == Some(revision)
                    && draft.pending_revision == Some(revision)
                    && draft.base_fingerprint == base_fingerprint
                    && draft.pending_claim_id.as_deref() == Some(claim_id)
                    && draft.pending_commands.is_some())
                .then_some(draft.mode == FlowIrDraftMode::Replace)
            })
        })
    }

    /// Clone the exact retained command batch only when the live board and every component of the
    /// pending delivery token still match. The caller must hold the live board lock from this call
    /// through application so the fingerprint cannot change between validation and mutation.
    /// This is intentionally non-consuming: a failed atomic apply may be retried or dismissed.
    pub fn pending_commands_if_current(
        &self,
        board: &Board,
        draft_id: &str,
        revision: u64,
        base_fingerprint: &str,
        claim_id: &str,
    ) -> Option<Vec<BoardCommand>> {
        if board_fingerprint(board) != base_fingerprint {
            return None;
        }
        let typed = self.drafts.lock().ok().and_then(|drafts| {
            let draft = drafts.get(draft_id.trim())?;
            (draft.revision == revision
                && draft.committed_revision == Some(revision)
                && draft.pending_revision == Some(revision)
                && draft.base_fingerprint == base_fingerprint
                && draft.pending_claim_id.as_deref() == Some(claim_id))
            .then(|| draft.pending_commands.clone())
            .flatten()
        });
        typed.or_else(|| {
            self.source_drafts.lock().ok().and_then(|drafts| {
                let draft = drafts.get(draft_id.trim())?;
                (draft.revision == revision
                    && draft.committed_revision == Some(revision)
                    && draft.pending_revision == Some(revision)
                    && draft.base_fingerprint == base_fingerprint
                    && draft.pending_claim_id.as_deref() == Some(claim_id))
                .then(|| draft.pending_commands.clone())
                .flatten()
            })
        })
    }

    /// Atomically dismiss the exact pending delivery generation. This avoids both a check/release
    /// race and the ABA case where a repeated disposition for an older token arrives after retry.
    pub fn release_commit_if_matches(
        &self,
        draft_id: &str,
        revision: u64,
        base_fingerprint: &str,
        claim_id: &str,
    ) -> bool {
        if let Ok(mut drafts) = self.drafts.lock()
            && let Some(draft) = drafts.get_mut(draft_id.trim())
        {
            if draft.revision != revision
                || draft.committed_revision != Some(revision)
                || draft.pending_revision != Some(revision)
                || draft.base_fingerprint != base_fingerprint
                || draft.pending_claim_id.as_deref() != Some(claim_id)
            {
                return false;
            }
            draft.committed_revision = None;
            draft.pending_revision = None;
            draft.pending_claim_id = None;
            draft.pending_commands = None;
            let state_sequence = self.next_access_sequence();
            draft.state_sequence = state_sequence;
            draft.access_sequence = state_sequence;
            return true;
        }
        let Ok(mut drafts) = self.source_drafts.lock() else {
            return false;
        };
        let Some(draft) = drafts.get_mut(draft_id.trim()) else {
            return false;
        };
        if draft.revision != revision
            || draft.committed_revision != Some(revision)
            || draft.pending_revision != Some(revision)
            || draft.base_fingerprint != base_fingerprint
            || draft.pending_claim_id.as_deref() != Some(claim_id)
        {
            return false;
        }
        draft.committed_revision = None;
        draft.pending_revision = None;
        draft.pending_claim_id = None;
        draft.pending_commands = None;
        let state_sequence = self.next_access_sequence();
        draft.state_sequence = state_sequence;
        draft.access_sequence = state_sequence;
        true
    }

    /// Preflight an Apply action against the exact pending batch and its original board revision.
    /// This rejects a stale review before any of its commands are executed.
    pub fn pending_commit_is_current(
        &self,
        board: &Board,
        draft_id: &str,
        revision: u64,
        base_fingerprint: &str,
        claim_id: &str,
    ) -> bool {
        let current_fingerprint = board_fingerprint(board);
        current_fingerprint == base_fingerprint
            && self.pending_commit_matches(draft_id, revision, base_fingerprint, claim_id)
    }

    /// Resolve a successfully applied batch. The live board must have advanced from the exact base
    /// carried by the review token; permanent idempotency remains in `committed_revision`.
    pub fn acknowledge_applied_commit(
        &self,
        board: &Board,
        draft_id: &str,
        revision: u64,
        base_fingerprint: &str,
        claim_id: &str,
    ) -> bool {
        let current_fingerprint = board_fingerprint(board);
        if current_fingerprint == base_fingerprint {
            return false;
        }
        if let Ok(mut drafts) = self.drafts.lock()
            && let Some(draft) = drafts.get_mut(draft_id.trim())
        {
            if draft.revision != revision
                || draft.committed_revision != Some(revision)
                || draft.pending_revision != Some(revision)
                || draft.base_fingerprint != base_fingerprint
                || draft.pending_claim_id.as_deref() != Some(claim_id)
            {
                return false;
            }
            draft.pending_revision = None;
            draft.pending_claim_id = None;
            draft.pending_commands = None;
            let state_sequence = self.next_access_sequence();
            draft.state_sequence = state_sequence;
            draft.access_sequence = state_sequence;
            return true;
        }
        let Ok(mut drafts) = self.source_drafts.lock() else {
            return false;
        };
        let Some(draft) = drafts.get_mut(draft_id.trim()) else {
            return false;
        };
        if draft.revision != revision
            || draft.committed_revision != Some(revision)
            || draft.pending_revision != Some(revision)
            || draft.base_fingerprint != base_fingerprint
            || draft.pending_claim_id.as_deref() != Some(claim_id)
        {
            return false;
        }
        draft.pending_revision = None;
        draft.pending_claim_id = None;
        draft.pending_commands = None;
        let state_sequence = self.next_access_sequence();
        draft.state_sequence = state_sequence;
        draft.access_sequence = state_sequence;
        true
    }

    /// Whether any retained draft has returned a command batch that the host has not rolled back.
    /// Host caches use this to keep the board-scoped store alive until the live board fingerprint
    /// advances, preserving board-wide idempotency across surface/LRU churn.
    pub fn has_pending_commit(&self) -> bool {
        self.drafts.lock().is_ok_and(|drafts| {
            drafts
                .values()
                .any(|draft| draft.pending_revision.is_some())
        }) || self.source_drafts.lock().is_ok_and(|drafts| {
            drafts
                .values()
                .any(|draft| draft.pending_revision.is_some())
        })
    }

    pub fn has_editable_draft_for_board(&self, board_id: &str) -> bool {
        self.drafts.lock().is_ok_and(|drafts| {
            drafts.values().any(|draft| {
                draft.board_id == board_id.trim() && draft.committed_revision.is_none()
            })
        }) || self.source_drafts.lock().is_ok_and(|drafts| {
            drafts.values().any(|draft| {
                draft.board_id == board_id.trim() && draft.committed_revision.is_none()
            })
        })
    }

    pub fn editable_flowscript_draft_recovery(
        &self,
        board: &Board,
        raw_request: &str,
    ) -> FlowScriptDraftRecovery {
        self.editable_flowscript_draft_recovery_for_identity(
            board,
            &FlowIrRequestIdentity::from_raw_request(raw_request),
        )
    }

    pub fn editable_flowscript_draft_recovery_for_binding(
        &self,
        board: &Board,
        binding: &FlowIrAcceptanceBinding,
    ) -> FlowScriptDraftRecovery {
        if binding.board_id != board.id {
            return FlowScriptDraftRecovery::none();
        }
        self.editable_flowscript_draft_recovery_for_identity(board, &binding.request_identity)
    }

    fn editable_flowscript_draft_recovery_for_identity(
        &self,
        board: &Board,
        request_identity: &FlowIrRequestIdentity,
    ) -> FlowScriptDraftRecovery {
        let candidates = self
            .source_drafts
            .lock()
            .ok()
            .map(|drafts| {
                drafts
                    .iter()
                    .filter(|(_, draft)| {
                        draft.board_id == board.id && draft.committed_revision.is_none()
                    })
                    .map(|(draft_id, draft)| (draft_id.clone(), draft.clone()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let exact = candidates
            .iter()
            .filter(|(_, draft)| &draft.request_identity == request_identity)
            .max_by_key(|(_, draft)| draft.access_sequence);
        if let Some((draft_id, draft)) = exact {
            let context =
                editable_flowscript_draft_context(board, draft_id.clone(), draft.clone(), true);
            if context.stale_board {
                return FlowScriptDraftRecovery {
                    status: FlowIrDraftRecoveryStatus::ExactMatch,
                    auto_resume: false,
                    exact_match: Some(context),
                    conflicting_draft: None,
                    next_actions: vec!["start_new_draft_from_current_board".to_string()],
                    message: "The retained source belongs to this exact request, but its board base is stale. Preserve it as reference and start a new draft from the current board; patch and commit are intentionally disabled."
                        .to_string(),
                };
            }
            return FlowScriptDraftRecovery {
                status: FlowIrDraftRecoveryStatus::ExactMatch,
                auto_resume: true,
                exact_match: Some(context),
                conflicting_draft: None,
                next_actions: vec!["resume_exact_flowscript_draft".to_string()],
                message: "The newest retained FlowScript source belongs to this exact immutable request and may be resumed at its retained revision."
                    .to_string(),
            };
        }
        let conflicting = candidates
            .into_iter()
            .max_by_key(|(_, draft)| draft.access_sequence);
        let Some((_draft_id, _draft)) = conflicting else {
            return FlowScriptDraftRecovery::none();
        };
        FlowScriptDraftRecovery {
            status: FlowIrDraftRecoveryStatus::RequestMismatch,
            auto_resume: false,
            exact_match: None,
            // A mismatch is only a host control-flow signal. Draft identifiers and revisions are
            // capabilities for the source lifecycle, so do not disclose them to another request.
            conflicting_draft: None,
            next_actions: vec![
                "recover_with_original_request".to_string(),
                "abandon_retained_draft_via_host".to_string(),
                "begin_separate_draft_for_current_request".to_string(),
            ],
            message: "A retained FlowScript source exists on this board, but it belongs to a different immutable request and was not auto-resumed."
                .to_string(),
        }
    }

    /// Resolve recovery against the immutable raw request. Only an exact normalized request hash
    /// can become `auto_resume`; a same-board mismatch is returned as non-authoritative metadata.
    pub fn editable_draft_recovery(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        raw_request: &str,
    ) -> FlowIrDraftRecovery {
        self.editable_draft_recovery_for_identity(
            board,
            catalog,
            &FlowIrRequestIdentity::from_raw_request(raw_request),
        )
    }

    pub fn editable_draft_recovery_for_binding(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        binding: &FlowIrAcceptanceBinding,
    ) -> FlowIrDraftRecovery {
        if binding.board_id != board.id {
            return FlowIrDraftRecovery::none();
        }
        self.editable_draft_recovery_for_identity(board, catalog, &binding.request_identity)
    }

    fn editable_draft_recovery_for_identity(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
        request_identity: &FlowIrRequestIdentity,
    ) -> FlowIrDraftRecovery {
        let candidates = self
            .drafts
            .lock()
            .ok()
            .map(|drafts| {
                drafts
                    .iter()
                    .filter(|(_, draft)| {
                        draft.board_id == board.id && draft.committed_revision.is_none()
                    })
                    .map(|(draft_id, draft)| (draft_id.clone(), draft.clone()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let exact = candidates
            .iter()
            .filter(|(_, draft)| &draft.request_identity == request_identity)
            .max_by_key(|(_, draft)| draft.access_sequence);
        if let Some((draft_id, draft)) = exact {
            return FlowIrDraftRecovery {
                status: FlowIrDraftRecoveryStatus::ExactMatch,
                auto_resume: true,
                exact_match: Some(editable_draft_context(
                    board,
                    catalog,
                    draft_id.clone(),
                    draft.clone(),
                )),
                conflicting_draft: None,
                next_actions: vec!["resume_exact_draft".to_string()],
                message: "The newest editable draft belongs to this exact normalized user request and may be resumed at its retained revision."
                    .to_string(),
            };
        }
        let conflicting = candidates
            .into_iter()
            .max_by_key(|(_, draft)| draft.access_sequence);
        let Some((draft_id, draft)) = conflicting else {
            return FlowIrDraftRecovery::none();
        };
        FlowIrDraftRecovery {
            status: FlowIrDraftRecoveryStatus::RequestMismatch,
            auto_resume: false,
            exact_match: None,
            conflicting_draft: Some(editable_draft_context(
                board, catalog, draft_id, draft,
            )),
            next_actions: vec![
                "recover_with_original_request".to_string(),
                "abandon_retained_draft_via_host".to_string(),
                "begin_separate_draft_for_current_request".to_string(),
            ],
            message: "An editable draft exists on this board, but it belongs to a different immutable user request. It was not auto-resumed and cannot be mutated or committed under the current request binding."
                .to_string(),
        }
    }

    /// Request-scoped authorization used by manual/provider tool dispatch before it touches an
    /// existing draft. Missing drafts proceed to the ordinary tool error; existing mismatches fail
    /// closed with explicit recover/abandon choices.
    pub fn authorize_draft_request(
        &self,
        board_id: &str,
        draft_id: &str,
        binding: Option<&FlowIrAcceptanceBinding>,
    ) -> Result<(), FlowIrDraftRequestMismatch> {
        let draft_id = draft_id.trim();
        let draft = self
            .drafts
            .lock()
            .ok()
            .and_then(|drafts| drafts.get(draft_id).cloned());
        let Some(draft) = draft else {
            let source = self
                .source_drafts
                .lock()
                .ok()
                .and_then(|drafts| drafts.get(draft_id).cloned());
            let Some(source) = source else {
                return Ok(());
            };
            return source_draft_request_authorization_error(board_id, draft_id, &source, binding)
                .map_or(Ok(()), Err);
        };
        let Some(binding) = binding else {
            return Err(draft_request_mismatch(
                draft_id,
                draft.revision,
                "IR_DRAFT_REQUEST_BINDING_REQUIRED",
                "An existing typed draft requires the host's immutable request binding before it can be resumed or committed.",
            ));
        };
        if binding.board_id != board_id.trim() || draft.board_id != board_id.trim() {
            return Err(draft_request_mismatch(
                draft_id,
                draft.revision,
                "IR_DRAFT_REQUEST_BOARD_MISMATCH",
                "The retained typed draft or request binding belongs to a different board.",
            ));
        }
        if draft.request_identity != binding.request_identity {
            return Err(draft_request_mismatch(
                draft_id,
                draft.revision,
                "IR_DRAFT_REQUEST_IDENTITY_MISMATCH",
                "This typed draft belongs to a different immutable user request. It was not resumed and no operation was dispatched.",
            ));
        }
        Ok(())
    }

    /// Host-only recovery hint for a fresh agent run. It exposes no queued commands or claim
    /// nonce; the resumed run must still validate and commit through the normal typed lifecycle.
    pub fn latest_editable_draft_context(
        &self,
        board: &Board,
        catalog: &[NodeMetadata],
    ) -> Option<FlowIrEditableDraftContext> {
        let (draft_id, draft) = self.drafts.lock().ok().and_then(|drafts| {
            drafts
                .iter()
                .filter(|(_, draft)| {
                    draft.board_id == board.id && draft.committed_revision.is_none()
                })
                .max_by_key(|(_, draft)| draft.access_sequence)
                .map(|(draft_id, draft)| (draft_id.clone(), draft.clone()))
        })?;
        Some(editable_draft_context(board, catalog, draft_id, draft))
    }

    pub fn latest_editable_flowscript_draft_context(
        &self,
        board: &Board,
    ) -> Option<FlowScriptEditableDraftContext> {
        let (draft_id, draft) = self.source_drafts.lock().ok().and_then(|drafts| {
            drafts
                .iter()
                .filter(|(_, draft)| {
                    draft.board_id == board.id && draft.committed_revision.is_none()
                })
                .max_by_key(|(_, draft)| draft.access_sequence)
                .map(|(draft_id, draft)| (draft_id.clone(), draft.clone()))
        })?;
        Some(editable_flowscript_draft_context(
            board, draft_id, draft, true,
        ))
    }

    /// Return the newest pending FlowScript review belonging to this exact immutable request.
    ///
    /// A current board receives the exact retained command batch. A stale board receives the same
    /// source and claim token with `stale_board = true` and no applicable commands, allowing the
    /// host to surface an explicit Dismiss-only review without stranding the claim. This accessor
    /// does not mutate access order or claim state, so a lost or aborted redelivery can safely retry
    /// and receive the identical source and claim nonce.
    pub fn pending_flowscript_delivery_for_binding(
        &self,
        board: &Board,
        binding: &FlowIrAcceptanceBinding,
    ) -> Option<FlowScriptPendingDelivery> {
        if binding.board_id != board.id {
            return None;
        }
        let current_fingerprint = board_fingerprint(board);
        self.source_drafts.lock().ok().and_then(|drafts| {
            drafts
                .iter()
                .filter_map(|(draft_id, draft)| {
                    if draft.board_id != board.id
                        || draft.request_identity != binding.request_identity
                        || draft.committed_revision != Some(draft.revision)
                        || draft.pending_revision != Some(draft.revision)
                    {
                        return None;
                    }
                    let claim_id = draft.pending_claim_id.as_ref()?;
                    let commands = draft.pending_commands.as_ref()?;
                    if commands.is_empty() {
                        return None;
                    }
                    let stale_board = draft.base_fingerprint != current_fingerprint;
                    Some((
                        draft.access_sequence,
                        FlowScriptPendingDelivery {
                            source: draft.source.clone(),
                            token: FlowIrCommitToken {
                                board_id: board.id.clone(),
                                draft_id: draft_id.clone(),
                                revision: draft.revision,
                                base_fingerprint: draft.base_fingerprint.clone(),
                                claim_id: claim_id.clone(),
                                requires_destructive_approval: draft.mode
                                    == FlowIrDraftMode::Replace,
                            },
                            stale_board,
                            commands: if stale_board {
                                Vec::new()
                            } else {
                                commands.clone()
                            },
                        },
                    ))
                })
                .max_by_key(|(access_sequence, _)| *access_sequence)
                .map(|(_, delivery)| delivery)
        })
    }

    /// Return the newest exact pending delivery for a host response without exposing its nonce to
    /// the model-visible tool result. The host must retain this store and use the token only with
    /// atomic Apply/Dismiss operations.
    pub fn latest_pending_commit_token(&self, board_id: &str) -> Option<FlowIrCommitToken> {
        if board_id.trim().is_empty() {
            return None;
        }
        let typed = self.drafts.lock().ok().and_then(|drafts| {
            drafts
                .iter()
                .filter_map(|(draft_id, draft)| {
                    if draft.board_id != board_id.trim() {
                        return None;
                    }
                    let revision = draft.pending_revision?;
                    let claim_id = draft.pending_claim_id.as_ref()?;
                    draft.pending_commands.as_ref()?;
                    (draft.committed_revision == Some(revision)).then(|| {
                        (
                            draft.access_sequence,
                            FlowIrCommitToken {
                                board_id: board_id.to_string(),
                                draft_id: draft_id.clone(),
                                revision,
                                base_fingerprint: draft.base_fingerprint.clone(),
                                claim_id: claim_id.clone(),
                                requires_destructive_approval: draft.mode
                                    == FlowIrDraftMode::Replace,
                            },
                        )
                    })
                })
                .max_by_key(|(access_sequence, _)| *access_sequence)
        });
        let source = self.source_drafts.lock().ok().and_then(|drafts| {
            drafts
                .iter()
                .filter_map(|(draft_id, draft)| {
                    if draft.board_id != board_id.trim() {
                        return None;
                    }
                    let revision = draft.pending_revision?;
                    let claim_id = draft.pending_claim_id.as_ref()?;
                    draft.pending_commands.as_ref()?;
                    (draft.committed_revision == Some(revision)).then(|| {
                        (
                            draft.access_sequence,
                            FlowIrCommitToken {
                                board_id: board_id.to_string(),
                                draft_id: draft_id.clone(),
                                revision,
                                base_fingerprint: draft.base_fingerprint.clone(),
                                claim_id: claim_id.clone(),
                                requires_destructive_approval: draft.mode
                                    == FlowIrDraftMode::Replace,
                            },
                        )
                    })
                })
                .max_by_key(|(access_sequence, _)| *access_sequence)
        });
        match (typed, source) {
            (Some(typed), Some(source)) => Some(if typed.0 >= source.0 {
                typed.1
            } else {
                source.1
            }),
            (Some((_, token)), None) | (None, Some((_, token))) => Some(token),
            (None, None) => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FlowIrEditableDraftContext {
    pub board_id: String,
    pub draft_id: String,
    pub revision: u64,
    pub status: String,
    pub base_fingerprint: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_modules: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remaining_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<FlowIrDiagnostic>,
}

fn evict_pending_contract_capacity(
    contracts: &mut HashMap<String, PendingRequestAcceptanceContract>,
) {
    if contracts.len() < MAX_FLOW_IR_ACCEPTANCE_CONTRACTS_PER_STORE {
        return;
    }
    let victim = contracts
        .iter()
        .filter(|(_, pending)| pending.claimed_draft_id.is_none())
        .min_by_key(|(_, pending)| pending.access_sequence)
        .map(|(id, _)| id.clone())
        .or_else(|| {
            contracts
                .iter()
                .min_by_key(|(_, pending)| pending.access_sequence)
                .map(|(id, _)| id.clone())
        });
    if let Some(victim) = victim {
        // If every slot is claimed, expiring the oldest handle is fail-closed: a late retry
        // receives IR_ACCEPTANCE_BINDING_INVALID instead of silently losing scope.
        contracts.remove(&victim);
    }
}

fn draft_request_mismatch(
    draft_id: &str,
    revision: u64,
    code: &'static str,
    message: &str,
) -> FlowIrDraftRequestMismatch {
    FlowIrDraftRequestMismatch {
        status: "request_identity_mismatch",
        code,
        retryable: false,
        auto_resume: false,
        draft_id: draft_id.to_string(),
        revision,
        next_actions: vec![
            "recover_with_original_request".to_string(),
            "abandon_retained_draft_via_host".to_string(),
            "begin_separate_draft_for_current_request".to_string(),
        ],
        message: message.to_string(),
    }
}

fn draft_request_authorization_error(
    board_id: &str,
    draft_id: &str,
    draft: &StoredDraft,
    binding: Option<&FlowIrAcceptanceBinding>,
) -> Option<FlowIrDraftRequestMismatch> {
    let binding = binding?;
    if binding.board_id != board_id.trim() || draft.board_id != board_id.trim() {
        return Some(draft_request_mismatch(
            draft_id,
            draft.revision,
            "IR_DRAFT_REQUEST_BOARD_MISMATCH",
            "The retained typed draft or request binding belongs to a different board.",
        ));
    }
    (draft.request_identity != binding.request_identity).then(|| {
        draft_request_mismatch(
            draft_id,
            draft.revision,
            "IR_DRAFT_REQUEST_IDENTITY_MISMATCH",
            "This typed draft belongs to a different immutable user request. It was not resumed and no operation was dispatched.",
        )
    })
}

fn source_draft_request_authorization_error(
    board_id: &str,
    draft_id: &str,
    draft: &StoredFlowScriptDraft,
    binding: Option<&FlowIrAcceptanceBinding>,
) -> Option<FlowIrDraftRequestMismatch> {
    let Some(binding) = binding else {
        return (draft.request_identity != FlowIrRequestIdentity::unbound()).then(|| {
            draft_request_mismatch(
                draft_id,
                draft.revision,
                "FLOWSCRIPT_DRAFT_REQUEST_BINDING_REQUIRED",
                "This retained FlowScript belongs to a bound immutable request and requires the host request binding.",
            )
        });
    };
    if binding.board_id != board_id.trim() || draft.board_id != board_id.trim() {
        return Some(draft_request_mismatch(
            draft_id,
            draft.revision,
            "FLOWSCRIPT_DRAFT_REQUEST_BOARD_MISMATCH",
            "The retained FlowScript draft or request binding belongs to a different board.",
        ));
    }
    (draft.request_identity != binding.request_identity).then(|| {
        draft_request_mismatch(
            draft_id,
            draft.revision,
            "FLOWSCRIPT_DRAFT_REQUEST_IDENTITY_MISMATCH",
            "This FlowScript draft belongs to a different immutable user request. It was not resumed or committed.",
        )
    })
}

fn flowscript_request_mismatch_response(
    denied: FlowIrDraftRequestMismatch,
) -> FlowScriptDraftResponse {
    // Do not use `for_draft` here. A request mismatch must not disclose or authorize the source,
    // draft id, revision, base fingerprint, diagnostics, or correction hints owned by the other
    // immutable request.
    let message = format!(
        "{} Begin a separate draft for the current request with a new draft id.",
        denied.message
    );
    let mut response = FlowScriptDraftResponse::error(denied.code, message);
    response.status = denied.status.to_string();
    response
}

/// Surface the prohibitions the machine could not enforce exactly where the batch is handed
/// onward, so the human review sees which bans it alone must verify.
fn append_omitted_prohibition_notice(message: &mut String, contract: &RequestAcceptanceContract) {
    if contract.omitted_prohibitions.is_empty() {
        return;
    }
    message.push_str(&format!(
        " {} user prohibition(s) could not be machine-enforced and must be verified in the human review: {}.",
        contract.omitted_prohibitions.len(),
        contract.omitted_prohibitions.join("; ")
    ));
}

fn flowscript_base_revision_conflict_response(
    draft_id: String,
    draft: &StoredFlowScriptDraft,
) -> FlowScriptDraftResponse {
    let mut response = FlowScriptDraftResponse::for_draft(
        "error",
        "The board changed after this source draft began. Preserve it as reference and start a new draft from the current board.",
        draft_id,
        draft,
    );
    response.code = Some("FLOWSCRIPT_BASE_REVISION_CONFLICT".to_string());
    response
}

fn flowscript_candidate_regression_response(
    draft_id: String,
    draft: &StoredFlowScriptDraft,
    regression: FlowScriptCandidateRegression,
) -> FlowScriptDraftResponse {
    let mut response = FlowScriptDraftResponse::for_draft(
        "error",
        format!(
            "Candidate regression blocked: the patch collapsed from {} to {} call sites and from {} to {} meaningful statements while retaining only {}/{} stable scope identities. Repair the retained source in place, or set allow_scope_reduction only for an explicit user-requested reduction.",
            regression.previous_call_sites,
            regression.candidate_call_sites,
            regression.previous_statements,
            regression.candidate_statements,
            regression.retained_scope_symbols,
            regression.previous_scope_symbols,
        ),
        draft_id,
        draft,
    );
    response.code = Some("FLOWSCRIPT_CANDIDATE_REGRESSION".to_string());
    response
}

fn evaluate_flowscript_source(
    board: &Board,
    catalog: &[NodeMetadata],
    source: &str,
    mode: FlowIrDraftMode,
    acceptance_contract: Option<&RequestAcceptanceContract>,
) -> EvaluatedFlowScriptSource {
    let mut acceptance_diagnostics = Vec::new();
    let reconcile = match flow_like_ast::parse(source) {
        Ok(ast) => {
            let reconcile =
                reconcile_with_catalog_mode(board, &ast, catalog, mode.reconcile_mode());
            // Do not pile semantic scope errors on top of syntax/catalog errors. A repaired source
            // is checked again against the same immutable contract, so this remains fail closed.
            if reconcile.diagnostics.is_empty()
                && let Some(contract) = acceptance_contract
            {
                acceptance_diagnostics = flowscript_acceptance_diagnostics(contract, &ast, catalog);
            }
            reconcile
        }
        Err(error) => ReconcileResult {
            commands: Vec::new(),
            corrections: Vec::new(),
            diagnostics: vec![format!(
                "FlowScript parse error at line {}, col {}: {}",
                error.line, error.col, error.message
            )],
        },
    };
    let mut diagnostics = reconcile.structured_diagnostics_for_source(source);
    enrich_flowscript_diagnostics_with_catalog(&mut diagnostics, catalog);
    // Only explicit prohibitions remain fail-closed. Incomplete-scope and approval-shape findings
    // come from a prose-derived heuristic with known false positives on correct scripts; they are
    // demoted to review notes so a converging repair loop can still commit.
    let (blocking, review_notes): (Vec<_>, Vec<_>) =
        acceptance_diagnostics.into_iter().partition(|diagnostic| {
            diagnostic.code == FlowScriptDiagnosticCode::FsRequestAcceptanceForbidden
        });
    diagnostics.extend(blocking);
    EvaluatedFlowScriptSource {
        diagnostics,
        review_notes,
        commands: reconcile.commands,
        corrections: reconcile.corrections,
    }
}

/// Attach the exact live-catalog evidence needed to repair catalog-related FlowScript failures.
///
/// `get_declarations` remains useful for planning, but a failed write/patch/check already has the
/// active catalog in memory and should not force the model to rediscover a known signature. For a
/// resolved call (for example, one with a misspelled pin), this supplies exactly that call's
/// declaration. For an unresolved call name, it supplies only a small deterministic set of close
/// symbol matches; candidates are evidence, not an automatic semantic substitution.
fn enrich_flowscript_diagnostics_with_catalog(
    diagnostics: &mut [FlowScriptDiagnostic],
    catalog: &[NodeMetadata],
) {
    for diagnostic in diagnostics {
        if !matches!(
            diagnostic.code,
            FlowScriptDiagnosticCode::FsCatalogDeclarationNotFound
                | FlowScriptDiagnosticCode::FsCatalogDeclarationAmbiguous
                | FlowScriptDiagnosticCode::FsUnknownInputPin
                | FlowScriptDiagnosticCode::FsUnresolvedArgument
                | FlowScriptDiagnosticCode::FsOutputPinUnresolved
                | FlowScriptDiagnosticCode::FsExecutionPolicyAmbiguous
                | FlowScriptDiagnosticCode::FsBranchArmPinUnknown
        ) {
            continue;
        }
        let Some(authored_name) = diagnostic.declaration.as_deref() else {
            continue;
        };
        let (declarations, companion_declarations, exact_match) =
            catalog_repair_declarations(catalog, authored_name, diagnostic.code);
        if declarations.is_empty() {
            continue;
        }

        let fix = diagnostic
            .fix
            .get_or_insert_with(|| FlowScriptDiagnosticFix {
                summary: "Repair this call using an exact declaration from the active catalog."
                    .to_string(),
                declaration_search: None,
                catalog_declarations: Vec::new(),
                companion_declarations: Vec::new(),
            });
        fix.declaration_search = None;
        fix.catalog_declarations = declarations;
        fix.companion_declarations = companion_declarations;
        fix.summary = if exact_match {
            "Patch the call to use the exact function and pin names in the supplied live-catalog declaration."
                .to_string()
        } else {
            "Choose a supplied live-catalog candidate only if it matches the intended operation, then patch the function name and all arguments to that exact signature."
                .to_string()
        };
    }
}

fn catalog_repair_declarations(
    catalog: &[NodeMetadata],
    authored_name: &str,
    code: FlowScriptDiagnosticCode,
) -> (Vec<String>, Vec<String>, bool) {
    let exact_metadata = catalog
        .iter()
        .filter(|metadata| {
            let signature = metadata_to_signature(metadata);
            signature.display == authored_name || metadata.name == authored_name
        })
        .collect::<Vec<_>>();
    let mut exact = exact_metadata
        .iter()
        .map(|metadata| compact_catalog_declaration(&metadata_to_signature(metadata)))
        .collect::<Vec<_>>();
    exact.sort();
    exact.dedup();
    exact.truncate(MAX_FLOWSCRIPT_REPAIR_DECLARATIONS);
    if !exact.is_empty() {
        let mut companion_declarations = Vec::new();
        for metadata in exact_metadata {
            for companion_name in &metadata.companion_nodes {
                let Some(companion) = unique_catalog_repair_companion(catalog, companion_name)
                else {
                    continue;
                };
                let declaration = compact_catalog_declaration(&metadata_to_signature(companion));
                if !companion_declarations.contains(&declaration) {
                    companion_declarations.push(declaration);
                }
                if companion_declarations.len() >= MAX_FLOWSCRIPT_REPAIR_COMPANION_DECLARATIONS {
                    break;
                }
            }
            if companion_declarations.len() >= MAX_FLOWSCRIPT_REPAIR_COMPANION_DECLARATIONS {
                break;
            }
        }
        return (exact, companion_declarations, true);
    }

    if code != FlowScriptDiagnosticCode::FsCatalogDeclarationNotFound {
        return (Vec::new(), Vec::new(), false);
    }
    let authored_symbol = normalize_catalog_symbol(authored_name);
    if authored_symbol.is_empty() {
        return (Vec::new(), Vec::new(), false);
    }

    let mut candidates = catalog
        .iter()
        .filter_map(|metadata| {
            let signature = metadata_to_signature(metadata);
            let candidate_symbol = normalize_catalog_symbol(&signature.display);
            let similarity = strsim::jaro_winkler(&authored_symbol, &candidate_symbol);
            (similarity >= MIN_FLOWSCRIPT_REPAIR_DECLARATION_SIMILARITY).then(|| {
                (
                    similarity,
                    signature.display.clone(),
                    compact_catalog_declaration(&signature),
                )
            })
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .0
            .total_cmp(&left.0)
            .then_with(|| left.1.cmp(&right.1))
    });

    let mut declarations = Vec::new();
    for (_, _, declaration) in candidates {
        if !declarations.contains(&declaration) {
            declarations.push(declaration);
        }
        if declarations.len() >= MAX_FLOWSCRIPT_REPAIR_DECLARATIONS {
            break;
        }
    }
    (declarations, Vec::new(), false)
}

/// Companion repair hints are labeled as exact live-catalog evidence, so they must satisfy the
/// same uniqueness rule as declaration lookup. Suppress both duplicate internal node types and
/// distinct node types that collide on one FlowScript display name.
fn unique_catalog_repair_companion<'a>(
    catalog: &'a [NodeMetadata],
    companion_name: &str,
) -> Option<&'a NodeMetadata> {
    let mut internal_matches = catalog
        .iter()
        .filter(|candidate| candidate.name == companion_name);
    let companion = internal_matches.next()?;
    if internal_matches.next().is_some() {
        return None;
    }
    let display = metadata_to_signature(companion).display;
    let mut display_matches = catalog
        .iter()
        .filter(|candidate| metadata_to_signature(candidate).display == display);
    display_matches.next()?;
    display_matches.next().is_none().then_some(companion)
}

fn normalize_catalog_symbol(symbol: &str) -> String {
    symbol
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn compact_catalog_declaration(signature: &flow_like_ast::Signature) -> String {
    let rendered = signature.render_declaration();
    let declaration = rendered
        .lines()
        .find(|line| line.trim_start().starts_with("declare function "))
        .map(str::trim)
        .unwrap_or_default();
    if signature.impure {
        format!("{declaration} // impure")
    } else {
        declaration.to_string()
    }
}

/// Host-only projection from parsed FlowScript into the existing representation-neutral request
/// evidence evaluator. This is not a model surface and is never retained or serialized: its sole
/// purpose is proving that reachable source code still covers the immutable raw request.
struct FlowScriptAcceptanceProjection<'a> {
    catalog: &'a [NodeMetadata],
    function_names: HashSet<String>,
    next_id: u64,
}

impl<'a> FlowScriptAcceptanceProjection<'a> {
    fn new(ast: &BoardAst, catalog: &'a [NodeMetadata]) -> Self {
        Self {
            catalog,
            function_names: ast
                .functions
                .iter()
                .map(|function| normalize(&function.name))
                .collect(),
            next_id: 0,
        }
    }

    fn project(mut self, ast: &BoardAst) -> FlowIrProgram {
        let variables = ast
            .variables
            .iter()
            .map(|variable| FlowIrVariable {
                name: variable.name.clone(),
                value_type: acceptance_projection_type(),
                default: variable.default.as_ref().map(project_ast_literal),
                exposed: variable.exposed,
                secret: variable.secret,
                editable: variable.editable,
                runtime_configured: variable.runtime_configured,
                category: variable.category.clone(),
                description: variable.description.clone(),
                anchor: variable.anchor.clone(),
            })
            .collect::<Vec<_>>();

        let mut modules = Vec::new();
        for (event_index, event) in ast.events.iter().enumerate() {
            modules.push(self.project_event(event, &format!("/events/{event_index}")));
        }
        for (function_index, function) in ast.functions.iter().enumerate() {
            modules.push(self.project_function(function, &format!("/functions/{function_index}")));
        }
        for (event_index, event) in ast.events.iter().enumerate() {
            self.project_handlers_in_block(
                &event.body,
                &format!("/events/{event_index}/body"),
                &mut modules,
            );
        }
        for (function_index, function) in ast.functions.iter().enumerate() {
            self.project_handlers_in_block(
                &function.body,
                &format!("/functions/{function_index}/body"),
                &mut modules,
            );
        }

        FlowIrProgram {
            interfaces: Vec::new(),
            variables,
            modules,
            ..Default::default()
        }
    }

    fn project_event(&mut self, event: &AstEventBlock, path: &str) -> FlowIrModule {
        let mut steps = Vec::new();
        self.project_block(&event.body, &format!("{path}/body"), &mut steps);
        FlowIrModule::Event {
            name: event.name.clone(),
            node_type: self.resolve_catalog_node_type(&event.node_type, &event.name),
            params: project_ast_params(&event.params),
            steps,
            anchor: event.anchor.clone(),
        }
    }

    fn project_function(&mut self, function: &AstFnDecl, path: &str) -> FlowIrModule {
        let mut steps = Vec::new();
        self.project_block(&function.body, &format!("{path}/body"), &mut steps);
        FlowIrModule::Function {
            name: function.name.clone(),
            params: project_ast_params(&function.params),
            returns: project_ast_params(&function.returns),
            steps,
            anchor: function.anchor.clone(),
        }
    }

    fn project_handlers_in_block(
        &mut self,
        block: &AstBlock,
        path: &str,
        modules: &mut Vec<FlowIrModule>,
    ) {
        for (index, statement) in block.stmts.iter().enumerate() {
            let statement_path = format!("{path}/stmts/{index}");
            match statement {
                AstStmt::Handler(handler) => {
                    modules.push(self.project_event(handler, &statement_path));
                    self.project_handlers_in_block(
                        &handler.body,
                        &format!("{statement_path}/handler/body"),
                        modules,
                    );
                }
                AstStmt::Branch { arms, .. } => {
                    for (arm_index, arm) in arms.iter().enumerate() {
                        self.project_handlers_in_block(
                            &arm.body,
                            &format!("{statement_path}/arms/{arm_index}/body"),
                            modules,
                        );
                    }
                }
                AstStmt::Loop { body, .. } => {
                    self.project_handlers_in_block(body, &format!("{statement_path}/body"), modules)
                }
                AstStmt::Let { .. }
                | AstStmt::Call { .. }
                | AstStmt::Assign { .. }
                | AstStmt::FieldAssign { .. }
                | AstStmt::LocalAlias { .. }
                | AstStmt::Return { .. }
                | AstStmt::Local(_)
                | AstStmt::Comment(_) => {}
            }
        }
    }

    fn project_block(&mut self, block: &AstBlock, path: &str, steps: &mut Vec<FlowIrStep>) {
        for (index, statement) in block.stmts.iter().enumerate() {
            let statement_path = format!("{path}/stmts/{index}");
            match statement {
                AstStmt::Let { name, call, anchor } => {
                    let step = self.project_call_step(
                        call,
                        name.clone(),
                        anchor.clone(),
                        &statement_path,
                        steps,
                    );
                    steps.push(step);
                }
                AstStmt::Call { call, anchor } => {
                    let id = self.fresh_id(&call.display);
                    let step =
                        self.project_call_step(call, id, anchor.clone(), &statement_path, steps);
                    steps.push(step);
                }
                AstStmt::Branch {
                    bind,
                    call,
                    condition,
                    arms,
                    anchor,
                } => {
                    let condition = if let Some(condition) = condition {
                        self.project_expr(condition, &format!("{statement_path}/condition"), steps)
                    } else if !call.display.trim().is_empty() || !call.node_type.trim().is_empty() {
                        let id = bind.clone().unwrap_or_else(|| self.fresh_id(&call.display));
                        let step = self.project_call_step(
                            call,
                            id.clone(),
                            anchor.clone(),
                            &format!("{statement_path}/branch_call"),
                            steps,
                        );
                        steps.push(step);
                        FlowIrValue::Output {
                            step: id,
                            pin: "decision".to_string(),
                            occurrence: 0,
                        }
                    } else if let Some(bind) = bind {
                        FlowIrValue::Ref { name: bind.clone() }
                    } else {
                        FlowIrValue::Literal {
                            value: FlowIrLiteral::Boolean(false),
                        }
                    };

                    let mut then_steps = Vec::new();
                    let mut else_steps = Vec::new();
                    for (arm_index, arm) in arms.iter().enumerate() {
                        let target = if approval_ast_arm_side(&arm.label, arm_index)
                            == ApprovalBranchSide::Then
                        {
                            &mut then_steps
                        } else {
                            &mut else_steps
                        };
                        self.project_block(
                            &arm.body,
                            &format!("{statement_path}/arms/{arm_index}/body"),
                            target,
                        );
                    }
                    steps.push(FlowIrStep::If {
                        id: bind
                            .clone()
                            .unwrap_or_else(|| self.fresh_id("approval_branch")),
                        condition,
                        then_steps,
                        else_steps,
                        anchor: anchor.clone(),
                    });
                }
                AstStmt::Loop {
                    keyword,
                    bind,
                    call,
                    body,
                    anchor,
                } => {
                    let loop_id = bind
                        .clone()
                        .unwrap_or_else(|| self.fresh_id(keyword.as_str()));
                    let call_value = self.project_call_value(
                        call,
                        format!("{loop_id}_source"),
                        anchor.clone(),
                        &format!("{statement_path}/loop_call"),
                        steps,
                    );
                    let mut body_steps = Vec::new();
                    self.project_block(body, &format!("{statement_path}/body"), &mut body_steps);
                    steps.push(FlowIrStep::ForEach {
                        id: loop_id,
                        array: call_value,
                        item: bind.clone().unwrap_or_else(|| "item".to_string()),
                        index: None,
                        parallel: keyword.eq_ignore_ascii_case("forEachParallel"),
                        steps: body_steps,
                        anchor: anchor.clone(),
                    });
                }
                AstStmt::Assign { target, value, .. } => {
                    let value = self.project_expr(value, &format!("{statement_path}/value"), steps);
                    steps.push(FlowIrStep::Assign {
                        target: target.clone(),
                        value,
                    });
                }
                AstStmt::FieldAssign {
                    base, path, value, ..
                } => {
                    let value = self.project_expr(value, &format!("{statement_path}/value"), steps);
                    steps.push(FlowIrStep::Assign {
                        target: format!("{base}.{path}"),
                        value,
                    });
                }
                AstStmt::LocalAlias { name, value, .. } => {
                    let value = self.project_expr(value, &format!("{statement_path}/value"), steps);
                    steps.push(FlowIrStep::Assign {
                        target: name.clone(),
                        value,
                    });
                }
                AstStmt::Return { values, .. } => {
                    let values = values
                        .iter()
                        .enumerate()
                        .map(|(value_index, value)| {
                            self.project_expr(
                                value,
                                &format!("{statement_path}/values/{value_index}"),
                                steps,
                            )
                        })
                        .collect();
                    steps.push(FlowIrStep::Return { values });
                }
                AstStmt::Local(variable) => {
                    if let Some(default) = variable.default.as_ref() {
                        steps.push(FlowIrStep::Assign {
                            target: variable.name.clone(),
                            value: FlowIrValue::Literal {
                                value: project_ast_literal(default),
                            },
                        });
                    }
                }
                // Nested handlers are projected as independent Event roots above. Comments have
                // no runtime semantics for request acceptance.
                AstStmt::Handler(_) | AstStmt::Comment(_) => {}
            }
        }
    }

    fn project_call_step(
        &mut self,
        call: &AstCall,
        id: String,
        anchor: Option<String>,
        path: &str,
        prefix_steps: &mut Vec<FlowIrStep>,
    ) -> FlowIrStep {
        let args = call
            .args
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                self.project_arg(argument, &format!("{path}/args/{index}"), prefix_steps)
            })
            .collect::<Vec<_>>();
        if self.function_names.contains(&normalize(&call.display)) {
            FlowIrStep::CallFunction {
                id,
                function: call.display.clone(),
                args,
                anchor,
            }
        } else {
            FlowIrStep::Node {
                id,
                node_type: self.resolve_catalog_node_type(&call.node_type, &call.display),
                args,
                continue_from: None,
                exec_arms: Vec::new(),
                anchor,
            }
        }
    }

    fn project_call_value(
        &mut self,
        call: &AstCall,
        id: String,
        anchor: Option<String>,
        path: &str,
        steps: &mut Vec<FlowIrStep>,
    ) -> FlowIrValue {
        let step = self.project_call_step(call, id.clone(), anchor, path, steps);
        steps.push(step);
        FlowIrValue::Output {
            step: id,
            pin: "value".to_string(),
            occurrence: 0,
        }
    }

    fn project_arg(
        &mut self,
        argument: &AstArg,
        path: &str,
        steps: &mut Vec<FlowIrStep>,
    ) -> FlowIrArg {
        FlowIrArg {
            pin: argument.name.clone(),
            occurrence: 0,
            value: self.project_expr(&argument.value, path, steps),
        }
    }

    fn project_expr(
        &mut self,
        expression: &AstExpr,
        path: &str,
        steps: &mut Vec<FlowIrStep>,
    ) -> FlowIrValue {
        if let Some((root, fields)) = ast_access_path(expression) {
            return if fields.is_empty() {
                FlowIrValue::Ref { name: root }
            } else {
                FlowIrValue::Output {
                    step: root,
                    pin: fields.join("."),
                    occurrence: 0,
                }
            };
        }
        match expression {
            AstExpr::Call(call) => {
                let id = self.fresh_id(&call.display);
                self.project_call_value(call, id, call.anchor.clone(), path, steps)
            }
            AstExpr::Object(fields) => FlowIrValue::Object {
                fields: fields
                    .iter()
                    .enumerate()
                    .map(|(index, field)| FlowIrObjectField {
                        key: field.key.clone(),
                        value: self.project_expr(
                            &field.value,
                            &format!("{path}/fields/{index}"),
                            steps,
                        ),
                    })
                    .collect(),
            },
            AstExpr::Array(items) => FlowIrValue::List {
                items: items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| {
                        self.project_expr(item, &format!("{path}/items/{index}"), steps)
                    })
                    .collect(),
            },
            AstExpr::Field { base, pin } => FlowIrValue::Object {
                fields: vec![
                    FlowIrObjectField {
                        key: "base".to_string(),
                        value: self.project_expr(base, &format!("{path}/base"), steps),
                    },
                    FlowIrObjectField {
                        key: "pin".to_string(),
                        value: FlowIrValue::Literal {
                            value: FlowIrLiteral::String(pin.clone()),
                        },
                    },
                ],
            },
            AstExpr::Member { base, field } => FlowIrValue::Object {
                fields: vec![
                    FlowIrObjectField {
                        key: "base".to_string(),
                        value: self.project_expr(base, &format!("{path}/base"), steps),
                    },
                    FlowIrObjectField {
                        key: "field".to_string(),
                        value: FlowIrValue::Literal {
                            value: FlowIrLiteral::String(field.clone()),
                        },
                    },
                ],
            },
            AstExpr::Index { base, index } => FlowIrValue::Object {
                fields: vec![
                    FlowIrObjectField {
                        key: "collection".to_string(),
                        value: self.project_expr(base, &format!("{path}/base"), steps),
                    },
                    FlowIrObjectField {
                        key: "index".to_string(),
                        value: self.project_expr(index, &format!("{path}/index"), steps),
                    },
                ],
            },
            AstExpr::Ternary {
                cond,
                then,
                otherwise,
            } => FlowIrValue::Object {
                fields: vec![
                    FlowIrObjectField {
                        key: "condition".to_string(),
                        value: self.project_expr(cond, &format!("{path}/condition"), steps),
                    },
                    FlowIrObjectField {
                        key: "then".to_string(),
                        value: self.project_expr(then, &format!("{path}/then"), steps),
                    },
                    FlowIrObjectField {
                        key: "otherwise".to_string(),
                        value: self.project_expr(otherwise, &format!("{path}/otherwise"), steps),
                    },
                ],
            },
            AstExpr::Binary { op, lhs, rhs } => FlowIrValue::Object {
                fields: vec![
                    FlowIrObjectField {
                        key: op.clone(),
                        value: FlowIrValue::Literal {
                            value: FlowIrLiteral::String(op.clone()),
                        },
                    },
                    FlowIrObjectField {
                        key: "left".to_string(),
                        value: self.project_expr(lhs, &format!("{path}/left"), steps),
                    },
                    FlowIrObjectField {
                        key: "right".to_string(),
                        value: self.project_expr(rhs, &format!("{path}/right"), steps),
                    },
                ],
            },
            AstExpr::Literal(literal) => FlowIrValue::Literal {
                value: project_ast_literal(literal),
            },
            AstExpr::Ref(_) => unreachable!("references are handled by ast_access_path"),
        }
    }

    fn resolve_catalog_node_type(&self, declared: &str, display: &str) -> String {
        if !declared.trim().is_empty() {
            return declared.to_string();
        }
        let display = normalize(display);
        let mut candidates = self.catalog.iter().filter(|metadata| {
            normalize(&metadata.name) == display || normalize(&metadata.friendly_name) == display
        });
        let Some(first) = candidates.next() else {
            return display.clone();
        };
        if candidates.next().is_some() {
            // Reconcile emits the authoritative ambiguity diagnostic. Returning the normalized
            // display here cannot accidentally pass check because acceptance runs only after a
            // diagnostic-free reconcile.
            display.clone()
        } else {
            first.name.clone()
        }
    }

    fn fresh_id(&mut self, hint: &str) -> String {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let hint = normalize(hint);
        if hint.is_empty() {
            format!("source_step_{id}")
        } else {
            format!("{hint}_{id}")
        }
    }
}

fn acceptance_projection_type() -> FlowIrType {
    FlowIrType {
        data_type: FlowIrDataType::Generic,
        container: FlowIrContainer::Normal,
        interface: None,
    }
}

fn project_ast_params(params: &[AstParam]) -> Vec<FlowIrParam> {
    params
        .iter()
        .map(|param| FlowIrParam {
            name: param.name.clone(),
            value_type: acceptance_projection_type(),
        })
        .collect()
}

fn project_ast_literal(literal: &AstLiteral) -> FlowIrLiteral {
    match literal {
        AstLiteral::String(value) => FlowIrLiteral::String(value.clone()),
        AstLiteral::Int(value) => FlowIrLiteral::Integer(*value),
        AstLiteral::Float(value) => FlowIrLiteral::Float(*value),
        AstLiteral::Bool(value) => FlowIrLiteral::Boolean(*value),
        AstLiteral::Null => FlowIrLiteral::Null,
        AstLiteral::Json(value) => serde_json::from_str(value)
            .map(FlowIrLiteral::Json)
            .unwrap_or_else(|_| FlowIrLiteral::String(value.clone())),
    }
}

fn ast_access_path(expression: &AstExpr) -> Option<(String, Vec<String>)> {
    match expression {
        AstExpr::Ref(name) => Some((name.clone(), Vec::new())),
        AstExpr::Field { base, pin } => {
            let (root, mut fields) = ast_access_path(base)?;
            fields.push(pin.clone());
            Some((root, fields))
        }
        AstExpr::Member { base, field } => {
            let (root, mut fields) = ast_access_path(base)?;
            fields.push(field.clone());
            Some((root, fields))
        }
        AstExpr::Index { .. }
        | AstExpr::Call(_)
        | AstExpr::Object(_)
        | AstExpr::Array(_)
        | AstExpr::Ternary { .. }
        | AstExpr::Binary { .. }
        | AstExpr::Literal(_) => None,
    }
}

fn approval_ast_arm_side(label: &str, index: usize) -> ApprovalBranchSide {
    let label = normalize(label);
    if label.contains("false")
        || label.contains("reject")
        || label.contains("change")
        || label.contains("error")
        || label.contains("failure")
        || label.contains("else")
    {
        ApprovalBranchSide::Else
    } else if label.contains("true")
        || label.contains("approve")
        || label.contains("success")
        || label.contains("then")
    {
        ApprovalBranchSide::Then
    } else if index == 0 {
        ApprovalBranchSide::Then
    } else {
        ApprovalBranchSide::Else
    }
}

fn flowscript_acceptance_diagnostics(
    contract: &RequestAcceptanceContract,
    ast: &BoardAst,
    catalog: &[NodeMetadata],
) -> Vec<FlowScriptDiagnostic> {
    let projection = FlowScriptAcceptanceProjection::new(ast, catalog).project(ast);
    acceptance_contract_diagnostics(contract, &projection, catalog)
        .into_iter()
        .map(project_acceptance_diagnostic_for_flowscript)
        .collect()
}

fn project_acceptance_diagnostic_for_flowscript(
    diagnostic: FlowIrDiagnostic,
) -> FlowScriptDiagnostic {
    let (code, message, fix) = match diagnostic.code.as_str() {
        "IR_REQUEST_ACCEPTANCE_CONTRACT_INCOMPLETE" => (
            FlowScriptDiagnosticCode::FsRequestAcceptanceIncomplete,
            format!(
                "The reachable FlowScript workflow does not implement required request scope {:?}.",
                diagnostic.scope.as_deref().unwrap_or("unspecified capability")
            ),
            "Add a reachable FlowScript call using a matching live declaration in an event or in a helper called from an event.",
        ),
        "IR_REQUEST_ACCEPTANCE_CONTRACT_FORBIDDEN" => (
            FlowScriptDiagnosticCode::FsRequestAcceptanceForbidden,
            format!(
                "The reachable FlowScript workflow implements prohibited request scope {:?}.",
                diagnostic.scope.as_deref().unwrap_or("unspecified capability")
            ),
            "Remove the prohibited reachable FlowScript call. An uncalled helper does not execute, but should also be removed when it is unnecessary.",
        ),
        _ => (
            FlowScriptDiagnosticCode::FsRequestApprovalInvalid,
            diagnostic.message.clone(),
            diagnostic.fix.as_deref().unwrap_or(
                "Repair the reachable FlowScript approval branch while preserving the exact reviewer and correlation values from the request.",
            ),
        ),
    };
    let id_material = format!(
        "{}|{}|{}|{}",
        code.as_str(),
        diagnostic.code,
        diagnostic.scope.as_deref().unwrap_or_default(),
        diagnostic.actual.as_deref().unwrap_or_default()
    );
    let digest = blake3::hash(id_material.as_bytes()).to_hex().to_string();
    FlowScriptDiagnostic {
        id: format!("FSD-{}", &digest[..16]),
        code,
        phase: FlowScriptDiagnosticPhase::Validation,
        message,
        source_span: None,
        ast_path: Some(if diagnostic.code.starts_with("IR_REQUEST_APPROVAL_") {
            "workflow.humanApprovalLoop".to_string()
        } else {
            "workflow.requestAcceptance".to_string()
        }),
        scope: diagnostic.scope,
        expected: diagnostic.expected,
        actual: diagnostic.actual,
        declaration: None,
        pin: None,
        fix: Some(FlowScriptDiagnosticFix {
            summary: fix.to_string(),
            declaration_search: None,
            catalog_declarations: Vec::new(),
            companion_declarations: Vec::new(),
        }),
        caused_by: None,
        occurrences: 1,
        related_messages: Vec::new(),
    }
}

fn retained_flowscript_candidate(
    source: &str,
    evaluation: &EvaluatedFlowScriptSource,
) -> RetainedFlowScriptCandidate {
    RetainedFlowScriptCandidate {
        source: source.to_string(),
        profile: profile_flowscript_candidate(source),
        parse_valid: flow_like_ast::parse(source).is_ok(),
        diagnostic_count: evaluation.diagnostics.len(),
    }
}

fn select_best_flowscript_candidate(
    previous: &RetainedFlowScriptCandidate,
    candidate: RetainedFlowScriptCandidate,
) -> RetainedFlowScriptCandidate {
    // Syntax validity is a hard quality boundary: a larger truncated draft must never become the
    // scope-regression baseline ahead of a parseable repair. Within the same validity tier, prefer
    // fewer diagnostics and use structural completeness only as the final tiebreaker. This mirrors
    // the legacy repair tracker's ordering while preserving the newer source on an exact tie.
    let candidate_is_better = if candidate.parse_valid != previous.parse_valid {
        candidate.parse_valid
    } else if candidate.diagnostic_count != previous.diagnostic_count {
        candidate.diagnostic_count < previous.diagnostic_count
    } else {
        candidate.profile.completeness_score() >= previous.profile.completeness_score()
    };
    if candidate_is_better {
        candidate
    } else {
        previous.clone()
    }
}

/// The most recent fully checked revision worth retaining across a mutating edit: the head check
/// itself when it is current, otherwise the salvage already carried by the draft.
fn salvageable_flowscript_revision(
    draft: &StoredFlowScriptDraft,
) -> Option<SalvageFlowScriptRevision> {
    match draft.checked.as_ref() {
        Some(checked)
            if checked.revision == draft.revision
                && checked.board_fingerprint == draft.base_fingerprint =>
        {
            Some(SalvageFlowScriptRevision {
                checked: checked.clone(),
                source: draft.source.clone(),
                evaluation: draft.evaluation.clone(),
            })
        }
        _ => draft.salvage.clone(),
    }
}

fn stored_flowscript_draft_size(draft: &StoredFlowScriptDraft) -> usize {
    draft
        .source
        .len()
        .saturating_add(draft.best_candidate.source.len())
        .saturating_add(encoded_json_size(&draft.evaluation.diagnostics))
        .saturating_add(encoded_json_size(&draft.evaluation.review_notes))
        .saturating_add(encoded_json_size(&draft.evaluation.corrections))
        .saturating_add(encoded_board_commands_size(&draft.evaluation.commands))
        .saturating_add(draft.checked.as_ref().map_or(0, |checked| {
            checked
                .catalog_fingerprint
                .len()
                .saturating_add(encoded_board_commands_size(&checked.commands))
        }))
        .saturating_add(draft.salvage.as_ref().map_or(0, |salvage| {
            salvage
                .source
                .len()
                .saturating_add(encoded_board_commands_size(&salvage.checked.commands))
        }))
        .saturating_add(
            draft
                .pending_commands
                .as_ref()
                .map_or(0, |commands| encoded_board_commands_size(commands)),
        )
        .saturating_add(2048)
}

fn encoded_board_commands_size(commands: &[BoardCommand]) -> usize {
    encoded_json_size(commands)
}

fn encoded_json_size<T: Serialize + ?Sized>(value: &T) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |encoded| encoded.len())
}

fn validate_flowscript_deletion_authorization(
    draft: &StoredFlowScriptDraft,
    checked: &CheckedFlowScriptRevision,
    args: &CommitFlowScriptArgs,
) -> Option<FlowScriptDraftResponse> {
    let requested_nodes = args
        .remove_node_ids
        .iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect::<HashSet<_>>();
    let requested_variables = args
        .remove_variable_ids
        .iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect::<HashSet<_>>();
    let requested_layers = args
        .remove_layer_ids
        .iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect::<HashSet<_>>();
    let requested_comments = args
        .remove_comment_ids
        .iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect::<HashSet<_>>();
    if draft.mode == FlowIrDraftMode::Additive
        && (args.allow_deletions
            || !requested_nodes.is_empty()
            || !requested_variables.is_empty()
            || !requested_layers.is_empty()
            || !requested_comments.is_empty())
    {
        let mut response = FlowScriptDraftResponse::for_draft(
            "error",
            "Additive FlowScript drafts cannot request deletions. Start a replace draft only for an explicit full-document replacement.",
            String::new(),
            draft,
        );
        response.draft_id = None;
        response.code = Some("FLOWSCRIPT_ADDITIVE_DELETION_INVALID".to_string());
        return Some(response);
    }

    let actual_nodes = checked
        .commands
        .iter()
        .filter_map(|command| match command {
            BoardCommand::RemoveNode { node_id, .. } => Some(node_id.clone()),
            _ => None,
        })
        .collect::<HashSet<_>>();
    let actual_variables = checked
        .commands
        .iter()
        .filter_map(|command| match command {
            BoardCommand::RemoveVariable { variable_id, .. } => Some(variable_id.clone()),
            _ => None,
        })
        .collect::<HashSet<_>>();
    let actual_layers = checked
        .commands
        .iter()
        .filter_map(|command| match command {
            BoardCommand::RemoveLayer { layer_id, .. } => Some(layer_id.clone()),
            _ => None,
        })
        .collect::<HashSet<_>>();
    let actual_comments = checked
        .commands
        .iter()
        .filter_map(|command| match command {
            BoardCommand::RemoveComment { comment_id, .. } => Some(comment_id.clone()),
            _ => None,
        })
        .collect::<HashSet<_>>();
    if draft.mode == FlowIrDraftMode::Replace
        && (requested_nodes != actual_nodes
            || requested_variables != actual_variables
            || requested_layers != actual_layers
            || requested_comments != actual_comments)
    {
        let mut nodes = actual_nodes.into_iter().collect::<Vec<_>>();
        let mut variables = actual_variables.into_iter().collect::<Vec<_>>();
        let mut layers = actual_layers.into_iter().collect::<Vec<_>>();
        let mut comments = actual_comments.into_iter().collect::<Vec<_>>();
        nodes.sort();
        variables.sort();
        layers.sort();
        comments.sort();
        let mut response = FlowScriptDraftResponse::for_draft(
            "error",
            format!(
                "Replacement derives exact node removals {nodes:?}, variable removals {variables:?}, layer removals {layers:?}, and comment removals {comments:?}; enumerate those ids exactly."
            ),
            String::new(),
            draft,
        );
        response.draft_id = None;
        response.code = Some("FLOWSCRIPT_DELETION_ALLOWLIST_MISMATCH".to_string());
        return Some(response);
    }
    let destructive = destructive_flowscript_command_summaries(&checked.commands);
    if draft.mode == FlowIrDraftMode::Additive && !destructive.is_empty() {
        let mut response = FlowScriptDraftResponse::for_draft(
            "error",
            format!(
                "Additive reconcile unexpectedly derived deletions: {}",
                destructive.join(", ")
            ),
            String::new(),
            draft,
        );
        response.draft_id = None;
        response.code = Some("FLOWSCRIPT_DELETION_NOT_ALLOWED".to_string());
        return Some(response);
    }
    None
}

fn draft_request_mismatch_response(denied: FlowIrDraftRequestMismatch) -> FlowIrDraftResponse {
    let mut response = FlowIrDraftResponse::error(
        denied.code,
        format!(
            "{} Begin a separate typed draft for the current request with a new draft id.",
            denied.message
        ),
    );
    response.status = denied.status.to_string();
    response
}

fn editable_flowscript_draft_context(
    board: &Board,
    draft_id: String,
    draft: StoredFlowScriptDraft,
    include_source: bool,
) -> FlowScriptEditableDraftContext {
    let stale_board = board_fingerprint(board) != draft.base_fingerprint;
    let checked = draft.checked.as_ref().is_some_and(|checked| {
        checked.revision == draft.revision
            && checked.board_fingerprint == draft.base_fingerprint
            && !stale_board
    });
    let status = if stale_board {
        "stale_board"
    } else if !draft.evaluation.diagnostics.is_empty() {
        "validation_errors"
    } else if checked {
        "valid"
    } else {
        "draft"
    };
    FlowScriptEditableDraftContext {
        board_id: draft.board_id,
        draft_id,
        revision: draft.revision,
        status: status.to_string(),
        base_fingerprint: draft.base_fingerprint,
        source: include_source.then_some(draft.source),
        diagnostics: draft.evaluation.diagnostics,
        checked,
        stale_board,
    }
}

fn editable_draft_context(
    board: &Board,
    _catalog: &[NodeMetadata],
    draft_id: String,
    draft: StoredDraft,
) -> FlowIrEditableDraftContext {
    let missing_modules = missing_modules(&draft);
    let stale_board = board_fingerprint(board) != draft.base_fingerprint;
    let cached_validation = draft
        .validated
        .as_ref()
        .filter(|validated| validated.revision == draft.revision);
    let remaining_capabilities = cached_validation
        .map(|validated| validated.remaining_capabilities.clone())
        .unwrap_or_else(|| pending_required_capability_ids(&draft.capability_request));
    let diagnostics = cached_validation
        .map(|validated| validated.diagnostics.clone())
        .unwrap_or_else(|| draft.staged_evaluation.diagnostics.clone());
    let status = if stale_board {
        "stale_board"
    } else if cached_validation.is_some_and(|validated| validated.valid) {
        "ready_to_commit"
    } else if draft.staged_evaluation.diagnostics.is_empty() && missing_modules.is_empty() {
        "ready_to_validate"
    } else {
        "editing"
    };
    FlowIrEditableDraftContext {
        board_id: board.id.clone(),
        draft_id,
        revision: draft.revision,
        status: status.to_string(),
        base_fingerprint: draft.base_fingerprint,
        missing_modules,
        remaining_capabilities,
        diagnostics: diagnostics.into_iter().take(12).collect(),
    }
}

fn evaluate_staged_program(
    catalog: &[NodeMetadata],
    program: FlowIrProgram,
    expected_modules: &HashMap<String, FlowModuleKind>,
) -> StagedDraftEvaluation {
    let compile = compile_flow_ir(&program, catalog);
    let mut diagnostics = compile.diagnostics.clone();
    diagnostics.extend(expected_module_diagnostics(&program, expected_modules));
    StagedDraftEvaluation {
        compile,
        diagnostics,
    }
}

fn pending_required_capability_ids(request: &FlowCapabilityPlanRequest) -> Vec<String> {
    let mut pending = request
        .requirements
        .iter()
        .filter(|requirement| requirement.required)
        .map(|requirement| requirement.id.clone())
        .collect::<Vec<_>>();
    pending.sort();
    pending.dedup();
    pending
}

fn staged_module_diagnostics(
    evaluation: &StagedDraftEvaluation,
    program: &FlowIrProgram,
    module_name: &str,
) -> Vec<FlowIrDiagnostic> {
    let normalized_name = normalize(module_name);
    let module_path = program
        .modules
        .iter()
        .position(|module| normalize(module.name()) == normalized_name)
        .map(|index| format!("/modules/{index}"));
    evaluation
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic
                .scope
                .as_deref()
                .is_some_and(|scope| normalize(scope) == normalized_name)
                || module_path.as_deref().is_some_and(|path| {
                    diagnostic.path == path || diagnostic.path.starts_with(&format!("{path}/"))
                })
        })
        .cloned()
        .collect()
}

fn evaluate_program(
    board: &Board,
    catalog: &[NodeMetadata],
    program: FlowIrProgram,
    capability_request: &FlowCapabilityPlanRequest,
    capability_plan: &FlowCapabilityPlan,
    expected_modules: &HashMap<String, FlowModuleKind>,
    mode: FlowIrDraftMode,
) -> EvaluatedDraft {
    let compile = compile_flow_ir(&program, catalog);
    let mut diagnostics = compile.diagnostics.clone();
    diagnostics.extend(expected_module_diagnostics(&program, expected_modules));
    let completion_diagnostics =
        validate_flow_capability_usage(&program, capability_request, capability_plan, catalog);
    let reconcile = compile.ast.as_ref().map(|ast| {
        let result = reconcile_with_catalog_mode(board, ast, catalog, mode.reconcile_mode());
        diagnostics.extend(
            result
                .structured_diagnostics_for_source(&compile.flowscript)
                .into_iter()
                .enumerate()
                .map(|(index, diagnostic)| FlowIrDiagnostic {
                    code: diagnostic.code.as_str().to_string(),
                    phase: format!("{:?}", diagnostic.phase).to_ascii_lowercase(),
                    path: diagnostic
                        .ast_path
                        .unwrap_or_else(|| format!("/reconcile/{index}")),
                    scope: diagnostic.scope,
                    message: diagnostic.message,
                    expected: diagnostic.expected,
                    actual: diagnostic.actual,
                    declaration: diagnostic.declaration,
                    pin: diagnostic.pin,
                    fix: diagnostic.fix.map(|fix| fix.summary),
                    caused_by: diagnostic.caused_by.into_iter().collect(),
                }),
        );
        result
    });
    EvaluatedDraft {
        compile,
        reconcile,
        completion_diagnostics,
        diagnostics,
    }
}

fn retained_draft_response(
    status: &str,
    code: &str,
    message: &str,
    draft_id: String,
    draft: &StoredDraft,
    evaluation: StagedDraftEvaluation,
    diagnostics: Vec<FlowIrDiagnostic>,
) -> FlowIrDraftResponse {
    FlowIrDraftResponse {
        status: status.to_string(),
        code: Some(code.to_string()),
        message: message.to_string(),
        draft_id: Some(draft_id),
        revision: Some(draft.revision),
        base_fingerprint: Some(draft.base_fingerprint.clone()),
        diagnostics,
        module_node_counts: evaluation.compile.module_node_counts,
        flowscript: Some(evaluation.compile.flowscript),
        retained_ir: None,
        capability_plan: Some(draft.capability_plan.clone()),
        remaining_capabilities: pending_required_capability_ids(&draft.capability_request),
        missing_modules: missing_modules(draft),
        derived_command_count: None,
        commands: Vec::new(),
    }
}

fn retained_staged_draft_response(
    status: &str,
    code: &str,
    message: &str,
    draft_id: String,
    draft: &StoredDraft,
    diagnostics: Vec<FlowIrDiagnostic>,
) -> FlowIrDraftResponse {
    retained_draft_response(
        status,
        code,
        message,
        draft_id,
        draft,
        draft.staged_evaluation.clone(),
        diagnostics,
    )
}

fn remaining_capability_ids(
    diagnostics: &[FlowIrDiagnostic],
    plan: Option<&FlowCapabilityPlan>,
) -> Vec<String> {
    let Some(plan) = plan else {
        return Vec::new();
    };
    let mut remaining = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "IR_REQUIRED_CAPABILITY_UNUSED")
        .filter_map(|diagnostic| {
            diagnostic
                .path
                .rsplit('/')
                .next()
                .and_then(|index| index.parse::<usize>().ok())
        })
        .filter_map(|index| plan.requirements.get(index))
        .filter(|resolution| resolution.required)
        .map(|resolution| resolution.id.clone())
        .collect::<Vec<_>>();
    remaining.sort();
    remaining.dedup();
    remaining
}

fn derive_request_acceptance_contract(prompt: &str) -> RequestAcceptanceContract {
    let bounded_prompt = prompt
        .chars()
        .take(MAX_FLOW_IR_ACCEPTANCE_PROMPT_CHARS)
        .collect::<String>();
    let clauses = explicit_request_clauses(&bounded_prompt);
    let mut criteria = Vec::new();
    let mut omitted_prohibitions: Vec<String> = Vec::new();
    let mut seen = HashSet::new();

    for (clause, explicit_list_item) in clauses {
        let semantic_clause = clause
            .to_ascii_lowercase()
            .find(" - über ")
            .filter(|_| clause.to_ascii_lowercase().contains("oder wie auch immer"))
            .map(|index| &clause[..index])
            .unwrap_or(&clause);
        let tokens = semantic_tokens(semantic_clause);
        let forbidden = clause_is_negated(semantic_clause, &tokens);
        let mut actions = tokens
            .iter()
            .filter_map(|token| canonical_action(token).map(str::to_string))
            .collect::<Vec<_>>();
        actions.sort();
        actions.dedup();
        let mut objects = salient_subject_terms(&tokens);
        if forbidden && objects.iter().any(|object| object == "cron_catalog") {
            objects.retain(|object| object != "schedule");
        } else if !forbidden {
            objects.retain(|object| object != "cron_catalog");
        }
        objects.sort();
        objects.dedup();

        // Prose is accepted only when it contains an explicit operation. A marked list is a
        // stronger host signal, so noun-only entries such as "Slack notification" are retained.
        if actions.is_empty() && (!explicit_list_item || objects.is_empty()) {
            continue;
        }
        // A prohibition without a host-recognized protocol/service subject is too ambiguous to
        // enforce from catalog metadata. Crucially, it is not turned into a positive requirement.
        if forbidden && objects.is_empty() {
            continue;
        }
        // Recipient-, timing-, or automation-scoped bans require dataflow/value inspection that
        // catalog metadata cannot prove. Enforcing them as a global protocol ban would make valid
        // approval flows impossible (for example, reviewer mail is allowed while automatic
        // customer mail is not), so omit them instead of weakening or inverting their meaning.
        // The omission is traced so the human review sees which bans were left to it.
        if forbidden && forbidden_scope_requires_dataflow(&tokens) {
            record_omitted_prohibition(&mut omitted_prohibitions, bounded_summary(&clause));
            continue;
        }
        // Generic requests such as "make it better" are not reliable enough to enforce.
        if objects.is_empty()
            && actions
                .iter()
                .all(|action| action == "create" || action == "change")
        {
            continue;
        }

        let semantic_variants = split_mail_protocol_semantics(actions, objects);
        for (actions, objects) in semantic_variants {
            let summary = bounded_summary(&clause);
            let singleton_subject = actions.is_empty()
                || objects
                    .iter()
                    .any(|object| matches!(object.as_str(), "schedule" | "cron_catalog"));
            // The summary is deliberately not part of the identity: two clauses that demand the
            // same actions on the same subjects are one criterion, even when their prose differs.
            let identity = if singleton_subject {
                format!("{}|singleton|{}", forbidden, objects.join(","))
            } else {
                format!("{}|{}|{}", forbidden, actions.join(","), objects.join(","))
            };
            if !seen.insert(identity) {
                continue;
            }
            criteria.push(RequestAcceptanceCriterion {
                summary,
                actions,
                objects,
                forbidden,
            });
            if criteria.len() >= MAX_FLOW_IR_ACCEPTANCE_CRITERIA {
                break;
            }
        }
        if criteria.len() >= MAX_FLOW_IR_ACCEPTANCE_CRITERIA {
            break;
        }
    }

    // A contract must never simultaneously require and ban the same scope. When a derived
    // prohibition collides with a derived requirement, the requirement wins: an inverted or
    // over-generalized ban would make the whole request unsatisfiable, while dropping it merely
    // defers that clause to human review (traced above). The collision test is action-aware: a
    // presence-only ban yields to any requirement touching its subjects, but an action-scoped ban
    // (for example "never SEND email") survives requirements that merely read the same subject
    // and yields only to a requirement demanding that same action on all of the banned subjects.
    let required_scopes = criteria
        .iter()
        .filter(|criterion| !criterion.forbidden)
        .map(|criterion| (criterion.actions.clone(), criterion.objects.clone()))
        .collect::<Vec<_>>();
    criteria.retain(|criterion| {
        if !criterion.forbidden {
            return true;
        }
        let contradicted = if forbidden_criterion_is_presence_only(criterion) {
            required_scopes.iter().any(|(_, objects)| {
                criterion
                    .objects
                    .iter()
                    .any(|object| objects.contains(object))
            })
        } else {
            required_scopes.iter().any(|(actions, objects)| {
                criterion
                    .actions
                    .iter()
                    .any(|action| actions.contains(action))
                    && criterion
                        .objects
                        .iter()
                        .all(|object| objects.contains(object))
            })
        };
        if contradicted {
            record_omitted_prohibition(&mut omitted_prohibitions, criterion.summary.clone());
        }
        !contradicted
    });

    // A single generic action is too ambiguous to enforce, but an explicit protocol/service such
    // as Slack, SMTP, or cron is a strong host-authored acceptance signal on its own.
    if criteria.len() < 2
        && !criteria.iter().any(|criterion| criterion.forbidden)
        && criteria
            .iter()
            .all(|criterion| criterion.objects.is_empty())
    {
        criteria.clear();
    }
    RequestAcceptanceContract {
        criteria,
        omitted_prohibitions,
        approval_loop: derive_request_approval_loop_contract(&bounded_prompt),
    }
}

/// Presence-only wording ("no cron node", "never use IMAP") bans a subject regardless of the
/// operation performed on it. A ban naming a concrete operation is action-scoped; generic
/// authoring verbs do not count as one.
fn forbidden_criterion_is_presence_only(criterion: &RequestAcceptanceCriterion) -> bool {
    criterion.actions.is_empty()
        || criterion
            .actions
            .iter()
            .all(|action| action == "create" || action == "change")
}

fn record_omitted_prohibition(omitted: &mut Vec<String>, summary: String) {
    if omitted.len() < MAX_FLOW_IR_ACCEPTANCE_CRITERIA && !omitted.contains(&summary) {
        omitted.push(summary);
    }
}

fn derive_request_approval_loop_contract(prompt: &str) -> Option<RequestApprovalLoopContract> {
    let lower = prompt.to_lowercase();
    let tokens = semantic_tokens(prompt);
    let has_approval = tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "approval"
                | "approve"
                | "approved"
                | "review"
                | "reviewer"
                | "confirmation"
                | "confirm"
                | "freigabe"
                | "freigeben"
                | "freigegeben"
                | "bestätigung"
                | "bestaetigung"
                | "bestötigung"
                | "bestätigen"
                | "bestaetigen"
        )
    });
    let has_branch = tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "if" | "when" | "else" | "otherwise" | "wenn" | "bei" | "sonst" | "ansonsten"
        )
    });
    let has_revision_or_reask = tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "change"
                | "changes"
                | "feedback"
                | "revise"
                | "revised"
                | "revision"
                | "regenerate"
                | "again"
                | "draft"
                | "anpassung"
                | "anpassen"
                | "angepasst"
                | "entwurf"
                | "erneut"
                | "wieder"
        )
    }) || lower.contains("re-send")
        || lower.contains("resend");
    let has_ui_action_contract = explicit_ui_approval_action_contract(&lower);
    // Separate approve/revise action routes are already an explicit branch contract even when the
    // delegated page instruction does not literally contain an "if" or "otherwise" sentence.
    if !(has_approval && has_revision_or_reask && (has_branch || has_ui_action_contract)) {
        return None;
    }

    Some(RequestApprovalLoopContract {
        reviewer_emails: reviewer_email_literals(prompt),
        channel: if has_ui_action_contract {
            RequestApprovalChannel::PageAction
        } else {
            RequestApprovalChannel::EmailReply
        },
    })
}

/// Detect the host-authored two-action page contract used by FlowPilot widgets. This must be
/// intentionally narrow: a request that merely mentions a dashboard while asking for approval by
/// email still belongs to the email-reply validator.
fn explicit_ui_approval_action_contract(lower_prompt: &str) -> bool {
    let has_ui_action_surface = [
        "page-action",
        "page action",
        "page_action",
        "eventsgeneric",
        "ui action",
        "button action",
        "form action",
    ]
    .iter()
    .any(|term| lower_prompt.contains(term));
    let has_approve_action = lower_prompt.contains("approve")
        || lower_prompt.contains("freigeben")
        || lower_prompt.contains("freigabe");
    let has_revise_action = lower_prompt.contains("revise")
        || lower_prompt.contains("revisionfeedback")
        || lower_prompt.contains("revision feedback")
        || lower_prompt.contains("überarbeitung")
        || lower_prompt.contains("ueberarbeitung");
    let carries_ticket = lower_prompt.contains("ticketid")
        || lower_prompt.contains("ticket_id")
        || lower_prompt.contains("ticket id");

    has_ui_action_surface && has_approve_action && has_revise_action && carries_ticket
}

fn reviewer_email_literals(prompt: &str) -> Vec<String> {
    let emails = extract_email_literals(prompt);
    if emails.len() <= 1 {
        return emails;
    }

    let lower = prompt.to_lowercase();
    let mut contextual = Vec::new();
    for email in &emails {
        let Some(index) = lower.find(email) else {
            continue;
        };
        let start = lower[..index]
            .char_indices()
            .rev()
            .nth(96)
            .map_or(0, |(offset, _)| offset);
        let end = lower[index..]
            .char_indices()
            .nth(96)
            .map_or(lower.len(), |(offset, _)| index + offset);
        let context = &lower[start..end];
        if [
            "reviewer",
            "review",
            "approval",
            "approve",
            "confirm",
            "christian",
            "freigabe",
            "bestätigung",
            "bestaetigung",
            "bestötigung",
            "prüfmail",
            "pruefmail",
        ]
        .iter()
        .any(|term| context.contains(term))
        {
            contextual.push(email.clone());
        }
    }
    contextual.sort();
    contextual.dedup();
    if contextual.is_empty() {
        emails
    } else {
        contextual
    }
}

fn extract_email_literals(value: &str) -> Vec<String> {
    fn valid_email(candidate: &str) -> bool {
        let mut parts = candidate.split('@');
        let Some(local) = parts.next() else {
            return false;
        };
        let Some(domain) = parts.next() else {
            return false;
        };
        parts.next().is_none()
            && !local.is_empty()
            && !domain.is_empty()
            && !local.starts_with('.')
            && !local.ends_with('.')
            && domain.contains('.')
            && !domain.starts_with('.')
            && !domain.ends_with('.')
            && !domain.starts_with('-')
            && !domain.ends_with('-')
    }

    let mut emails = Vec::new();
    let mut candidate = String::new();
    for character in value.chars().chain(std::iter::once(' ')) {
        if character.is_ascii_alphanumeric()
            || matches!(character, '.' | '_' | '%' | '+' | '-' | '@')
        {
            candidate.push(character.to_ascii_lowercase());
        } else if !candidate.is_empty() {
            let trimmed = candidate.trim_matches('.').to_string();
            if valid_email(&trimmed) {
                emails.push(trimmed);
            }
            candidate.clear();
        }
    }
    emails.sort();
    emails.dedup();
    emails
}

fn split_mail_protocol_semantics(
    actions: Vec<String>,
    objects: Vec<String>,
) -> Vec<(Vec<String>, Vec<String>)> {
    if objects.iter().any(|object| object == "schedule") && objects.len() > 1 {
        let mut schedule_actions = actions
            .iter()
            .filter(|action| matches!(action.as_str(), "schedule" | "trigger" | "create"))
            .cloned()
            .collect::<Vec<_>>();
        if schedule_actions.is_empty() {
            schedule_actions.push("schedule".to_string());
        }
        let mut other_objects = objects;
        other_objects.retain(|object| object != "schedule");
        let mut variants = vec![(schedule_actions, vec!["schedule".to_string()])];
        variants.extend(split_mail_protocol_semantics(actions, other_objects));
        return variants;
    }

    if !objects.iter().any(|object| object == "imap")
        || !objects.iter().any(|object| object == "smtp")
    {
        return vec![(actions, objects)];
    }

    let mut imap_objects = objects.clone();
    imap_objects.retain(|object| object != "smtp");
    let mut smtp_objects = objects;
    smtp_objects.retain(|object| object != "imap");
    let mut imap_actions = actions
        .iter()
        .filter(|action| action.as_str() == "read" || action.as_str() == "receive")
        .cloned()
        .collect::<Vec<_>>();
    if imap_actions.is_empty() {
        imap_actions.push("read".to_string());
    }
    let mut smtp_actions = actions
        .into_iter()
        .filter(|action| action == "send" || action == "call")
        .collect::<Vec<_>>();
    if smtp_actions.is_empty() {
        smtp_actions.push("send".to_string());
    }
    vec![(imap_actions, imap_objects), (smtp_actions, smtp_objects)]
}

fn clause_is_negated(clause: &str, tokens: &[String]) -> bool {
    // An exclusivity marker states HOW something must be done ("IMAP nur über Secrets",
    // "only via env variables"). Embedded negations then ban a practice, not the subject.
    if clause_has_exclusivity_marker(clause) {
        return false;
    }
    if let Some((prefix, tail)) = clause.split_once(',') {
        let prefix_tokens = semantic_tokens(prefix);
        if prefix_tokens.first().is_some_and(|token| {
            matches!(
                token.as_str(),
                "if" | "when" | "unless" | "falls" | "sofern" | "wenn" | "bei"
            )
        }) {
            let tail_tokens = semantic_tokens(tail);
            if tail_tokens.iter().any(|token| is_negation_token(token)) {
                return clause_is_negated(tail, &tail_tokens);
            }
        }
    }

    tokens.iter().enumerate().any(|(index, token)| {
        if !is_negation_token(token) {
            return false;
        }
        // "niemals hardcoden", "keine Credentials erfinden", "nicht als Anweisung befolgen":
        // the negation bans a manner/practice, so the clause stays a positive requirement.
        if negation_targets_manner(tokens, index) {
            return false;
        }

        let conditional_start = tokens[..index].iter().rposition(|candidate| {
            matches!(
                candidate.as_str(),
                "if" | "when" | "unless" | "falls" | "sofern" | "wenn" | "bei"
            )
        });
        let directive_after_condition = conditional_start.is_some_and(|start| {
            tokens[start + 1..index].iter().any(|candidate| {
                matches!(
                    candidate.as_str(),
                    "do" | "must"
                        | "should"
                        | "shall"
                        | "soll"
                        | "sollen"
                        | "darf"
                        | "dürfen"
                        | "duerfen"
                        | "muss"
                        | "müssen"
                        | "muessen"
                )
            })
        });

        // A negated condition still introduces a positive branch: "if not approved, revise and
        // send another review email". Only a directive after the condition makes it a ban, as in
        // "when processing mail, do not send customer email".
        conditional_start.is_none() || directive_after_condition
    })
}

fn clause_has_exclusivity_marker(clause: &str) -> bool {
    let lower = clause.to_lowercase();
    [
        "ausschließlich",
        "ausschliesslich",
        "nur über",
        "nur ueber",
        "nur via",
        "nur mit",
        "only via",
        "only through",
        "only over",
        "only with",
        "exclusively via",
        "exclusively through",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

/// True when the first relevant term after a negation is a manner/practice word rather than a
/// protocol/service subject. A salient subject seen first keeps the negation a genuine ban, and
/// so does a transfer/exfiltration action in the negation's scope: "Sende niemals Credentials
/// per E-Mail" bans the send itself, not merely a hardcoding practice, and must never be
/// whitelisted into a positive requirement. The scope is the negated clause tail plus the two
/// tokens directly before the negation, which covers verb-first imperatives in both languages.
fn negation_targets_manner(tokens: &[String], negation_index: usize) -> bool {
    let scoped_transfer = tokens[negation_index.saturating_sub(2)..negation_index]
        .iter()
        .chain(&tokens[negation_index + 1..])
        .any(|token| is_transfer_action_token(token));
    // "mail"/"email" are usually the salient subject noun ("Anweisungen in E-Mails nicht
    // befolgen"), so they count as the transfer verb only directly after the negation, as in
    // "never email credentials".
    let adjacent_mail_verb = tokens
        .get(negation_index + 1)
        .is_some_and(|token| matches!(token.as_str(), "mail" | "mails" | "email" | "emails"));
    if scoped_transfer || adjacent_mail_verb {
        return false;
    }
    for token in tokens[negation_index + 1..].iter().take(3) {
        if is_manner_practice_token(token) {
            return true;
        }
        if !salient_subject_terms(std::slice::from_ref(token)).is_empty() {
            return false;
        }
    }
    false
}

/// Actions that move data to an external party. A negation covering one of these is a genuine
/// prohibition even when a practice noun such as "credentials" follows it directly.
fn is_transfer_action_token(token: &str) -> bool {
    matches!(
        token,
        "send"
            | "sends"
            | "sent"
            | "sending"
            | "sende"
            | "senden"
            | "sendet"
            | "gesendet"
            | "versende"
            | "versenden"
            | "versendet"
            | "verschicke"
            | "verschicken"
            | "verschickt"
            | "schicke"
            | "schicken"
            | "geschickt"
            | "share"
            | "shares"
            | "shared"
            | "sharing"
            | "teile"
            | "teilen"
            | "geteilt"
            | "upload"
            | "uploads"
            | "uploaded"
            | "uploading"
            | "hochlade"
            | "hochladen"
            | "hochgeladen"
            | "publish"
            | "publishes"
            | "published"
            | "publishing"
            | "veröffentliche"
            | "veröffentlichen"
            | "veröffentlicht"
            | "veroeffentliche"
            | "veroeffentlichen"
            | "veroeffentlicht"
            | "post"
            | "posts"
            | "posted"
            | "posting"
            | "poste"
            | "posten"
            | "gepostet"
            | "maile"
            | "mailen"
            | "mailed"
            | "mailing"
            | "gemailt"
            | "emailed"
            | "emailing"
    )
}

fn is_manner_practice_token(token: &str) -> bool {
    matches!(
        token,
        "hardcode"
            | "hardcoded"
            | "hardcoden"
            | "hardcodieren"
            | "hardcodiert"
            | "erfinden"
            | "erfindet"
            | "erfunden"
            | "invent"
            | "invented"
            | "inventing"
            | "befolgen"
            | "befolgt"
            | "follow"
            | "followed"
            | "following"
            | "anweisung"
            | "anweisungen"
            | "instruction"
            | "instructions"
            | "credential"
            | "credentials"
            | "zugangsdaten"
    )
}

fn is_negation_token(token: &str) -> bool {
    matches!(
        token,
        "no" | "not"
            | "never"
            | "without"
            | "kein"
            | "keine"
            | "keinen"
            | "keinem"
            | "keiner"
            | "niemals"
            | "nicht"
            | "ohne"
    )
}

fn forbidden_scope_requires_dataflow(tokens: &[String]) -> bool {
    tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "automatic"
                | "automatically"
                | "automatisch"
                | "directly"
                | "direkt"
                | "customer"
                | "customers"
                | "kunde"
                | "kunden"
                | "kundin"
                | "kundinnen"
                | "recipient"
                | "requester"
                | "until"
                | "before"
                | "bevor"
                | "erst"
        )
    })
}

fn salient_subject_terms(tokens: &[String]) -> Vec<String> {
    tokens
        .iter()
        .flat_map(|token| match token.as_str() {
            "slack" => vec!["slack"],
            "teams" | "msteams" | "microsoftteams" => vec!["teams"],
            "discord" => vec!["discord"],
            "email" | "emails" | "mail" | "mails" | "kundenmail" | "kundenemail"
            | "freigabemail" | "freigabemails" => vec!["email"],
            "imap" => vec!["imap"],
            "smtp" => vec!["smtp"],
            "http" | "https" => vec!["http"],
            "webhook" | "webhooks" => vec!["webhook"],
            "json" => vec!["json"],
            "xml" => vec!["xml"],
            "csv" => vec!["csv"],
            "sql" => vec!["sql"],
            "db" | "database" | "databases" | "datenbank" | "datenbanken" => {
                vec!["database"]
            }
            "ai" | "ki" | "llm" | "model" | "models" | "modell" | "modelle" => {
                vec!["model"]
            }
            "pdf" => vec!["pdf"],
            "s3" => vec!["s3"],
            "cron" | "cronjob" | "cronjobs" | "scheduler" => {
                vec!["schedule", "cron_catalog"]
            }
            "schedule" | "zeitplan" | "zeitplanung" | "zeitgesteuert" | "zeitgesteuerte" => {
                vec!["schedule"]
            }
            _ => match canonical_object_term(token).as_deref() {
                Some("email") => vec!["email"],
                Some("database") => vec!["database"],
                Some("model") => vec!["model"],
                _ => Vec::new(),
            },
        })
        .map(str::to_string)
        .collect()
}

fn explicit_request_clauses(prompt: &str) -> Vec<(String, bool)> {
    let list_items = prompt
        .lines()
        .filter_map(strip_list_marker)
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();
    if list_items.len() >= 2 {
        return list_items
            .into_iter()
            .flat_map(|item| {
                split_request_sentences(item)
                    .into_iter()
                    .flat_map(|sentence| split_action_sequence(&sentence))
                    .map(|clause| (clause, true))
            })
            .collect();
    }

    split_request_sentences(prompt)
        .into_iter()
        .flat_map(|sentence| {
            split_action_sequence(&sentence)
                .into_iter()
                .map(|clause| (clause, false))
        })
        .collect()
}

fn strip_list_marker(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    for marker in ["- ", "* ", "+ ", "• ", "[ ] ", "[x] ", "[X] "] {
        if let Some(item) = trimmed.strip_prefix(marker) {
            return Some(item);
        }
    }
    let digit_count = trimmed
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .count();
    if digit_count == 0 {
        return None;
    }
    let rest = &trimmed[digit_count..];
    rest.strip_prefix(". ")
        .or_else(|| rest.strip_prefix(") "))
        .or_else(|| rest.strip_prefix(": "))
}

fn split_request_sentences(prompt: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut current = String::new();
    let mut characters = prompt.chars().peekable();
    while let Some(character) = characters.next() {
        let email_domain_dot = character == '.'
            && current
                .split_whitespace()
                .next_back()
                .is_some_and(|token| token.contains('@'))
            && characters
                .peek()
                .is_some_and(|next| next.is_ascii_alphanumeric());
        if matches!(character, ';' | '.' | '!' | '?' | '\n' | '\r') && !email_domain_dot {
            let sentence = current.trim();
            if !sentence.is_empty() {
                sentences.push(sentence.to_string());
            }
            current.clear();
        } else {
            current.push(character);
        }
    }
    let sentence = current.trim();
    if !sentence.is_empty() {
        sentences.push(sentence.to_string());
    }
    sentences
}

fn split_action_sequence(clause: &str) -> Vec<String> {
    fn split_recursive(clause: &str, output: &mut Vec<String>) {
        let lower = clause.to_ascii_lowercase();
        let mut boundaries = lower
            .match_indices(',')
            .map(|(index, delimiter)| (index, index + delimiter.len(), false))
            .collect::<Vec<_>>();
        for (delimiter, allow_german_verb_final) in [
            (" and then ", false),
            (" then ", false),
            (" and ", false),
            (" und dann ", true),
            (" dann ", true),
            (" und ", true),
            (" ansonsten ", true),
            (" sonst ", true),
            ("->", true),
            ("→", true),
            ("⇒", true),
        ] {
            boundaries.extend(lower.match_indices(delimiter).map(|(index, _)| {
                (
                    index,
                    index.saturating_add(delimiter.len()),
                    allow_german_verb_final,
                )
            }));
        }
        boundaries.sort_by_key(|(left_end, _, _)| *left_end);
        for (left_end, right_start, allow_german_verb_final) in boundaries {
            let left = clause[..left_end].trim();
            let right = clause[right_start..].trim();
            let comma_before_german_branch = clause[left_end..right_start].contains(',')
                && semantic_tokens(right).first().is_some_and(|token| {
                    matches!(token.as_str(), "sonst" | "ansonsten" | "danach")
                });
            if has_action(left)
                && (starts_with_action(right)
                    || (allow_german_verb_final || comma_before_german_branch)
                        && has_german_action(right))
            {
                split_recursive(left, output);
                split_recursive(right, output);
                return;
            }
        }
        let clause = clause.trim().trim_matches(',').trim();
        if !clause.is_empty() {
            output.push(clause.to_string());
        }
    }

    let mut output = Vec::new();
    split_recursive(clause, &mut output);
    output
}

fn has_action(value: &str) -> bool {
    semantic_tokens(value)
        .iter()
        .any(|token| canonical_action(token).is_some())
}

fn starts_with_action(value: &str) -> bool {
    semantic_tokens(value)
        .into_iter()
        .skip_while(|token| {
            matches!(
                token.as_str(),
                "and"
                    | "also"
                    | "then"
                    | "next"
                    | "finally"
                    | "afterward"
                    | "afterwards"
                    | "please"
                    | "it"
                    | "should"
                    | "must"
                    | "will"
                    | "und"
                    | "dann"
                    | "danach"
                    | "anschließend"
                    | "anschliessend"
                    | "schließlich"
                    | "schliesslich"
                    | "erneut"
                    | "wieder"
                    | "sonst"
                    | "ansonsten"
                    | "bitte"
                    | "soll"
                    | "muss"
            )
        })
        .next()
        .is_some_and(|token| canonical_action(&token).is_some())
}

fn has_german_action(value: &str) -> bool {
    semantic_tokens(value)
        .iter()
        .any(|token| canonical_german_action(token).is_some())
}

fn semantic_tokens(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut previous_was_lowercase = false;
    for character in value.chars() {
        if character.is_alphanumeric() {
            if character.is_uppercase() && previous_was_lowercase && !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            current.extend(character.to_lowercase());
            previous_was_lowercase = character.is_lowercase();
        } else {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            previous_was_lowercase = false;
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn canonical_action(token: &str) -> Option<&'static str> {
    if let Some(action) = canonical_german_action(token) {
        return Some(action);
    }
    Some(match token {
        "create" | "creates" | "created" | "creating" | "build" | "builds" | "built"
        | "building" | "make" | "makes" | "made" | "making" | "add" | "adds" | "added"
        | "adding" => "create",
        "change" | "changes" | "changed" | "changing" => "change",
        "fetch" | "fetches" | "fetched" | "fetching" | "get" | "gets" | "got" | "getting"
        | "read" | "reads" | "reading" | "retrieve" | "retrieves" | "retrieved" | "retrieving"
        | "load" | "loads" | "loaded" | "loading" | "list" | "lists" | "listed" | "listing"
        | "search" | "searches" | "searched" | "searching" | "query" | "queries" | "queried"
        | "querying" => "read",
        "receive" | "receives" | "received" | "receiving" | "accept" | "accepts" | "accepted"
        | "accepting" | "ingest" | "ingests" | "ingested" | "ingesting" | "submit" | "submits"
        | "submitted" | "submitting" => "receive",
        "parse" | "parses" | "parsed" | "parsing" | "decode" | "decodes" | "decoded"
        | "decoding" => "parse",
        "validate" | "validates" | "validated" | "validating" | "verify" | "verifies"
        | "verified" | "verifying" | "check" | "checks" | "checked" | "checking" => "validate",
        "transform" | "transforms" | "transformed" | "transforming" | "convert" | "converts"
        | "converted" | "converting" | "format" | "formats" | "formatted" | "formatting" => {
            "transform"
        }
        "process" | "processes" | "processed" | "processing" => "process",
        "send" | "sends" | "sent" | "sending" | "notify" | "notifies" | "notified"
        | "notifying" | "emailed" | "emailing" | "alert" | "alerts" | "alerted" | "alerting"
        | "respond" | "responds" | "responded" | "responding" | "return" | "returns"
        | "returned" | "returning" => "send",
        "save" | "saves" | "saved" | "saving" | "store" | "stores" | "stored" | "storing"
        | "write" | "writes" | "wrote" | "written" | "writing" | "persist" | "persists"
        | "persisted" | "persisting" | "insert" | "inserts" | "inserted" | "inserting" => "write",
        "update" | "updates" | "updated" | "updating" | "modify" | "modifies" | "modified"
        | "modifying" | "patch" | "patches" | "patched" | "patching" => "update",
        "delete" | "deletes" | "deleted" | "deleting" | "remove" | "removes" | "removed"
        | "removing" => "delete",
        "branch" | "branches" | "branched" | "branching" | "route" | "routes" | "routed"
        | "routing" | "switch" | "switches" | "switched" | "switching" => "branch",
        "iterate" | "iterates" | "iterated" | "iterating" | "loop" | "loops" | "looped"
        | "looping" => "iterate",
        "retry" | "retries" | "retried" | "retrying" | "repeat" | "repeats" | "repeated"
        | "repeating" => "retry",
        "wait" | "waits" | "waited" | "waiting" | "delay" | "delays" | "delayed" | "delaying" => {
            "wait"
        }
        "trigger" | "triggers" | "triggered" | "triggering" | "start" | "starts" | "started"
        | "starting" => "trigger",
        "schedule" | "schedules" | "scheduled" | "scheduling" => "schedule",
        "call" | "calls" | "called" | "calling" | "invoke" | "invokes" | "invoked" | "invoking"
        | "requested" | "requesting" => "call",
        "upload" | "uploads" | "uploaded" | "uploading" => "upload",
        "download" | "downloads" | "downloaded" | "downloading" => "download",
        "log" | "logs" | "logged" | "logging" | "audit" | "audits" | "audited" | "auditing"
        | "recorded" | "recording" => "log",
        "extract" | "extracts" | "extracted" | "extracting" => "extract",
        "filter" | "filters" | "filtered" | "filtering" => "filter",
        "sort" | "sorts" | "sorted" | "sorting" => "sort",
        "aggregate" | "aggregates" | "aggregated" | "aggregating" => "aggregate",
        "merge" | "merges" | "merged" | "merging" => "merge",
        "split" | "splits" | "splitting" => "split",
        "compare" | "compares" | "compared" | "comparing" => "compare",
        "classify" | "classifies" | "classified" | "classifying" => "classify",
        "summarize" | "summarizes" | "summarized" | "summarizing" | "summarise" | "summarises"
        | "summarised" | "summarising" => "summarize",
        "generate" | "generates" | "generated" | "generating" => "generate",
        "publish" | "publishes" | "published" | "publishing" => "publish",
        "approve" | "approves" | "approved" | "approving" => "approve",
        "reject" | "rejects" | "rejected" | "rejecting" => "reject",
        "authenticate" | "authenticates" | "authenticated" | "authenticating" => "authenticate",
        "authorize" | "authorizes" | "authorized" | "authorizing" | "authorise" | "authorises"
        | "authorised" | "authorising" => "authorize",
        "encrypt" | "encrypts" | "encrypted" | "encrypting" => "encrypt",
        "decrypt" | "decrypts" | "decrypted" | "decrypting" => "decrypt",
        _ => return None,
    })
}

fn canonical_german_action(token: &str) -> Option<&'static str> {
    Some(match token {
        "bau" | "baue" | "baut" | "bauen" | "gebaut" | "erzeuge" | "erzeugen" | "erzeugt"
        | "erzeugte" | "erstell" | "erstelle" | "erstellen" | "erstellt" | "erstelltet" => "create",
        "änder" | "ändere" | "ändern" | "ändert" | "geändert" | "aender" | "aendere"
        | "aendern" | "aendert" | "geaendert" => "change",
        "abruf" | "abrufe" | "abrufen" | "abgerufen" | "ruf" | "rufe" | "ruft" | "abfragen"
        | "abfrage" | "abgefragt" | "lese" | "lesen" | "lies" | "liest" | "gelesen" | "hole"
        | "holen" | "holt" | "geholt" => "read",
        "empfange" | "empfangen" | "empfängt" | "empfaengt" | "empfangenes" | "eingehend"
        | "eingehende" | "eingehenden" => "receive",
        "parse" | "parsen" | "geparst" | "dekodiere" | "dekodieren" | "dekodiert" => "parse",
        "prüfe" | "prüfen" | "prüft" | "geprüft" | "pruefe" | "pruefen" | "prueft" | "geprueft"
        | "kontrolliere" | "kontrollieren" | "kontrolliert" => "validate",
        "verarbeite" | "verarbeiten" | "verarbeitet" | "verarbeitung" => "process",
        "formatiere" | "formatieren" | "formatiert" | "wandle" | "wandeln" | "umwandeln"
        | "umgewandelt" => "transform",
        "sende" | "senden" | "sendet" | "gesendet" | "versende" | "versenden" | "versendet"
        | "verschicke" | "verschicken" | "verschickt" | "schicke" | "schicken" | "geschickt"
        | "maile" | "mailen" | "gemailt" | "benachrichtige" | "benachrichtigen"
        | "benachrichtigt" | "antworte" | "antworten" | "antwortet" | "geantwortet" => "send",
        "speichere" | "speichern" | "speichert" | "gespeichert" | "sichere" | "sichern"
        | "gesichert" | "persistiere" | "persistieren" | "persistiert" => "write",
        "überarbeite" | "überarbeiten" | "überarbeitet" | "ueberarbeite" | "ueberarbeiten"
        | "ueberarbeitet" | "überarbeitung" | "ueberarbeitung" | "passe" | "anpassen"
        | "angepasst" | "anpassung" | "aktualisiere" | "aktualisieren" | "aktualisiert" => "update",
        "lösche" | "löschen" | "gelöscht" | "loesche" | "loeschen" | "geloescht" | "entferne"
        | "entfernen" | "entfernt" => "delete",
        "verzweige" | "verzweigen" | "verzweigt" | "route" | "routen" | "geroutet"
        | "unterscheide" | "unterscheiden" | "unterschieden" => "branch",
        "wiederhole" | "wiederholen" | "wiederholt" | "wiederholung" => "retry",
        "warte" | "warten" | "wartet" | "gewartet" | "verzögere" | "verzögern" | "verzoegere"
        | "verzoegern" => "wait",
        "starte" | "starten" | "startet" | "gestartet" | "löse" | "lösen" | "auslösen"
        | "ausgelöst" | "loese" | "loesen" | "ausloesen" | "ausgeloest" => "trigger",
        "plane" | "planen" | "geplant" | "terminiere" | "terminieren" | "terminiert" => "schedule",
        "aufrufen" | "aufgerufen" | "anfragen" | "angefragt" | "bitten" | "gebeten" => "call",
        "protokolliere" | "protokollieren" | "protokolliert" | "logge" | "loggen" | "geloggt" => {
            "log"
        }
        "extrahiere" | "extrahieren" | "extrahiert" => "extract",
        "filtere" | "filtern" | "gefiltert" | "ignoriere" | "ignorieren" | "ignoriert" => "filter",
        "sortiere" | "sortieren" | "sortiert" => "sort",
        "vergleiche" | "vergleichen" | "verglichen" => "compare",
        "erkenne" | "erkennen" | "erkannt" | "klassifiziere" | "klassifizieren"
        | "klassifiziert" => "classify",
        "fasse" | "zusammenfassen" | "zusammengefasst" => "summarize",
        "generiere" | "generieren" | "generiert" | "beantworte" | "beantworten" | "beantwortet" => {
            "generate"
        }
        "veröffentliche" | "veröffentlichen" | "veröffentlicht" | "veroeffentliche"
        | "veroeffentlichen" | "veroeffentlicht" => "publish",
        "genehmige" | "genehmigen" | "genehmigt" | "freigeben" | "freigegeben" | "bestätige"
        | "bestätigen" | "bestätigt" | "bestaetige" | "bestaetigen" | "bestaetigt"
        | "bestötigung" | "bestaetigung" | "bestätigung" => "approve",
        "lehne" | "ablehnen" | "abgelehnt" | "verwerfe" | "verwerfen" | "verworfen" => "reject",
        "markiere" | "markieren" | "markiert" => "update",
        "isoliere" | "isolieren" | "isoliert" => "branch",
        "verhindere" | "verhindern" | "verhindert" => "validate",
        _ => return None,
    })
}

fn canonical_object_term(token: &str) -> Option<String> {
    if matches!(
        token,
        "a" | "an"
            | "and"
            | "also"
            | "as"
            | "at"
            | "be"
            | "by"
            | "each"
            | "every"
            | "finally"
            | "for"
            | "from"
            | "in"
            | "into"
            | "it"
            | "its"
            | "next"
            | "of"
            | "on"
            | "or"
            | "please"
            | "should"
            | "that"
            | "the"
            | "their"
            | "them"
            | "then"
            | "this"
            | "to"
            | "using"
            | "via"
            | "when"
            | "where"
            | "which"
            | "will"
            | "with"
            | "workflow"
            | "workflows"
            | "flow"
            | "flows"
            | "board"
            | "boards"
            | "node"
            | "nodes"
            | "thing"
            | "things"
            | "something"
            | "app"
            | "application"
            | "eine"
            | "einer"
            | "einem"
            | "einen"
            | "ein"
            | "der"
            | "die"
            | "das"
            | "den"
            | "dem"
            | "des"
            | "und"
            | "oder"
            | "dann"
            | "danach"
            | "anschließend"
            | "anschliessend"
            | "schließlich"
            | "schliesslich"
            | "erneut"
            | "wieder"
            | "wenn"
            | "sonst"
            | "ansonsten"
            | "bei"
            | "mit"
            | "ohne"
            | "von"
            | "vom"
            | "zu"
            | "zum"
            | "zur"
            | "für"
            | "fuer"
            | "aus"
            | "auf"
            | "als"
            | "wird"
            | "werden"
            | "ist"
            | "sind"
            | "soll"
            | "sollen"
            | "muss"
            | "müssen"
            | "muessen"
            | "nur"
            | "genau"
            | "sich"
            | "selbst"
            | "nach"
            | "vor"
            | "noch"
            | "nicht"
            | "wie"
            | "immer"
            | "mir"
    ) {
        return None;
    }
    if token.chars().count() < 3 && !matches!(token, "ai" | "db" | "id" | "ui") {
        return None;
    }
    if let Some(translated) = match token {
        "mail" | "mails" | "email" | "emails" | "e-mail" | "e-mails" => Some("email"),
        "kunde" | "kunden" | "kundin" | "kundinnen" | "kundenmail" | "kundenemail" => {
            Some("customer")
        }
        "freigabe" | "freigaben" | "freigabemail" | "freigabemails" => Some("approval"),
        "entwurf" | "entwürfe" | "entwuerfe" | "antwortentwurf" => Some("draft"),
        "modell" | "modelle" => Some("model"),
        "antwort" | "antworten" | "beantwortung" => Some("response"),
        "anfrage" | "anfragen" | "supportanfrage" | "supportanfragen" => Some("request"),
        "nachricht" | "nachrichten" => Some("message"),
        "datenbank" | "datenbanken" => Some("database"),
        "fehler" => Some("error"),
        _ => None,
    } {
        return Some(translated.to_string());
    }
    let mut canonical = token.to_string();
    if canonical.len() > 4 && canonical.ends_with('s') && !canonical.ends_with("ss") {
        canonical.pop();
    }
    Some(canonical)
}

fn acceptance_contract_diagnostics(
    contract: &RequestAcceptanceContract,
    program: &FlowIrProgram,
    catalog: &[NodeMetadata],
) -> Vec<FlowIrDiagnostic> {
    let catalog_by_name = catalog
        .iter()
        .map(|metadata| (normalize(&metadata.name), metadata))
        .collect::<HashMap<_, _>>();
    let reachable_semantics = reachable_flow_ir_occurrences(program)
        .into_iter()
        .filter_map(|occurrence| match occurrence {
            ReachableFlowIrOccurrence::CatalogNode(node_type) => catalog_by_name
                .get(&node_type)
                .copied()
                .map(catalog_acceptance_semantics),
            ReachableFlowIrOccurrence::BuiltInAction(action) => {
                Some((HashSet::from([action]), HashSet::new()))
            }
        })
        .collect::<Vec<_>>();
    let reachable_actions = reachable_semantics
        .iter()
        .flat_map(|(actions, _)| actions.iter().copied())
        .collect::<HashSet<_>>();
    let reachable_subjects = reachable_semantics
        .iter()
        .flat_map(|(_, subjects)| subjects.iter().cloned())
        .collect::<HashSet<_>>();

    let positive_criteria = contract
        .criteria
        .iter()
        .enumerate()
        .filter(|(_, criterion)| !criterion.forbidden)
        .collect::<Vec<_>>();
    let candidate_occurrences = positive_criteria
        .iter()
        .map(|(_, criterion)| {
            reachable_semantics
                .iter()
                .enumerate()
                .filter_map(|(index, (actions, subjects))| {
                    ((criterion.actions.is_empty()
                        || criterion
                            .actions
                            .iter()
                            .any(|action| acceptance_action_covered(action, actions)))
                        && criterion
                            .objects
                            .iter()
                            .all(|subject| subjects.contains(subject)))
                    .then_some(index)
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let mut occurrence_owner = vec![None; reachable_semantics.len()];
    let mut matched_criteria = vec![false; contract.criteria.len()];
    for criterion_index in 0..positive_criteria.len() {
        let mut visited = vec![false; reachable_semantics.len()];
        if assign_acceptance_occurrence(
            criterion_index,
            &candidate_occurrences,
            &mut occurrence_owner,
            &mut visited,
        ) {
            matched_criteria[positive_criteria[criterion_index].0] = true;
        }
    }
    let mut diagnostics = contract
        .criteria
        .iter()
        .enumerate()
        .filter_map(|(criterion_index, criterion)| {
            let violated = if criterion.forbidden {
                // Presence-only wording ("no cron node" / "do not create a cron node") is
                // intentionally subject based. Operational bans also require the forbidden
                // action, so an IMAP read does not violate "do not send email".
                let presence_only = criterion.actions.is_empty()
                    || criterion.actions.iter().all(|action| action == "create");
                !criterion.objects.is_empty()
                    && reachable_semantics.iter().any(|(actions, subjects)| {
                        criterion
                            .objects
                            .iter()
                            .all(|subject| subjects.contains(subject))
                            && (presence_only
                                || criterion
                                    .actions
                                    .iter()
                                    .any(|action| acceptance_action_covered(action, actions)))
                    })
            } else {
                !matched_criteria[criterion_index]
            };
            if !violated {
                return None;
            }
            let (code, message, expected, actual, fix) = if criterion.forbidden {
                (
                    "IR_REQUEST_ACCEPTANCE_CONTRACT_FORBIDDEN",
                    format!(
                        "The reachable Event call graph implements prohibited user-authored scope {:?}.",
                        criterion.summary
                    ),
                    "no reachable catalog declaration with the prohibited protocol/service semantics"
                        .to_string(),
                    format!(
                        "reachable catalog semantics include subject(s) [{}]",
                        criterion.objects.join(", ")
                    ),
                    "Remove the prohibited reachable declaration; an uncalled helper is not execution scope but should also be deleted if unnecessary."
                        .to_string(),
                )
            } else {
                (
                    "IR_REQUEST_ACCEPTANCE_CONTRACT_INCOMPLETE",
                    format!(
                        "The reachable Event call graph does not implement user-authored scope {:?}.",
                        criterion.summary
                    ),
                    format!(
                        "reachable catalog metadata covering action(s) [{}] and every salient subject [{}]",
                        criterion.actions.join(", "),
                        criterion.objects.join(", ")
                    ),
                    format!(
                        "reachable actions [{}], subjects [{}]",
                        sorted_terms(&reachable_actions),
                        sorted_owned_terms(&reachable_subjects)
                    ),
                    "Use a matching live catalog declaration in an Event or a function actually called from an Event. Model-authored capability prose and unreachable helper layers do not count."
                        .to_string(),
                )
            };
            Some(FlowIrDiagnostic {
                code: code.to_string(),
                phase: "validate".to_string(),
                path: "/modules".to_string(),
                scope: Some(criterion.summary.clone()),
                message,
                expected: Some(expected),
                actual: Some(actual),
                declaration: None,
                pin: None,
                fix: Some(fix),
                caused_by: Vec::new(),
            })
        })
        .collect::<Vec<_>>();
    if let Some(approval_loop) = &contract.approval_loop {
        diagnostics.extend(match approval_loop.channel {
            RequestApprovalChannel::EmailReply => {
                approval_loop_diagnostics(approval_loop, program, &catalog_by_name)
            }
            RequestApprovalChannel::PageAction => {
                ui_approval_loop_diagnostics(approval_loop, program, &catalog_by_name)
            }
        });
    }
    diagnostics
}

/// A call to a FlowScript/typed-IR function executes its body at the call site with params bound
/// to the caller's arguments. Evidence collection therefore inlines reachable function bodies,
/// cycle-guarded per call chain and capped at this depth.
const MAX_APPROVAL_FUNCTION_INLINE_DEPTH: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApprovalBranchSide {
    Then,
    Else,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ApprovalBranchMembership {
    if_index: usize,
    side: ApprovalBranchSide,
}

#[derive(Debug, Clone, Default)]
struct ApprovalValueEvidence {
    terms: HashSet<String>,
    /// Normalized names of runtime refs/outputs whose values flow into this expression. Literal
    /// placeholders such as "<ticket_id>" deliberately never enter this set.
    dynamic_keys: HashSet<String>,
    literal_emails: HashSet<String>,
}

impl ApprovalValueEvidence {
    fn merge(&mut self, other: &Self) {
        self.terms.extend(other.terms.iter().cloned());
        self.dynamic_keys.extend(other.dynamic_keys.iter().cloned());
        self.literal_emails
            .extend(other.literal_emails.iter().cloned());
    }

    fn add_label(&mut self, value: &str) {
        self.terms.extend(semantic_tokens(value));
    }

    fn add_dynamic(&mut self, value: &str) {
        self.add_label(value);
        let key = normalize(value);
        if !key.is_empty() {
            self.dynamic_keys.insert(key);
        }
    }
}

#[derive(Debug, Clone)]
struct ApprovalNodeEvidence {
    module_index: usize,
    sequence: usize,
    is_email_send: bool,
    is_draft_generation: bool,
    args: ApprovalValueEvidence,
    recipient: ApprovalValueEvidence,
    branches: Vec<ApprovalBranchMembership>,
}

#[derive(Debug, Clone)]
struct ApprovalIfEvidence {
    index: usize,
    path: String,
    condition: ApprovalValueEvidence,
    condition_has_runtime_input: bool,
}

#[derive(Debug, Default)]
struct ApprovalFlowEvidence {
    nodes: Vec<ApprovalNodeEvidence>,
    ifs: Vec<ApprovalIfEvidence>,
    next_sequence: usize,
}

struct ApprovalEvidenceCollector<'a> {
    catalog_by_name: &'a HashMap<String, &'a NodeMetadata>,
    functions: HashMap<String, &'a FlowIrModule>,
    flow: ApprovalFlowEvidence,
    module_index: usize,
}

impl<'a> ApprovalEvidenceCollector<'a> {
    fn collect_steps(
        &mut self,
        steps: &[FlowIrStep],
        path: &str,
        symbols: &mut HashMap<String, ApprovalValueEvidence>,
        outputs: &mut HashMap<String, ApprovalValueEvidence>,
        branches: &mut Vec<ApprovalBranchMembership>,
        visiting: &mut HashSet<String>,
    ) -> ApprovalValueEvidence {
        let mut returned = ApprovalValueEvidence::default();
        for (step_index, step) in steps.iter().enumerate() {
            let step_path = format!("{path}/steps/{step_index}");
            match step {
                FlowIrStep::Node {
                    id,
                    node_type,
                    args,
                    exec_arms,
                    ..
                } => {
                    let args_evidence = approval_args_evidence(args, symbols, outputs);
                    let recipient = approval_recipient_evidence(args, symbols, outputs);
                    let (is_email_send, is_draft_generation) = self
                        .catalog_by_name
                        .get(&normalize(node_type))
                        .map(|metadata| {
                            let (actions, subjects) = catalog_acceptance_semantics(metadata);
                            (
                                acceptance_action_covered("send", &actions)
                                    && (subjects.contains("email") || subjects.contains("smtp")),
                                acceptance_action_covered("generate", &actions)
                                    && subjects.contains("model"),
                            )
                        })
                        .unwrap_or((false, false));
                    self.flow.nodes.push(ApprovalNodeEvidence {
                        module_index: self.module_index,
                        sequence: self.flow.next_sequence,
                        is_email_send,
                        is_draft_generation,
                        args: args_evidence.clone(),
                        recipient,
                        branches: branches.clone(),
                    });
                    self.flow.next_sequence += 1;

                    let mut output_evidence = args_evidence;
                    output_evidence.add_dynamic(id);
                    output_evidence.add_label(node_type);
                    outputs.insert(normalize(id), output_evidence);

                    for (arm_index, arm) in exec_arms.iter().enumerate() {
                        let mut arm_symbols = symbols.clone();
                        let mut arm_outputs = outputs.clone();
                        self.collect_steps(
                            &arm.steps,
                            &format!("{step_path}/exec_arms/{arm_index}"),
                            &mut arm_symbols,
                            &mut arm_outputs,
                            branches,
                            visiting,
                        );
                    }
                }
                FlowIrStep::CallFunction {
                    id, function, args, ..
                } => {
                    let args_evidence = approval_args_evidence(args, symbols, outputs);
                    let key = normalize(function);
                    let mut call_output = args_evidence.clone();
                    call_output.add_dynamic(id);
                    call_output.add_label(function);
                    if visiting.len() < MAX_APPROVAL_FUNCTION_INLINE_DEPTH
                        && visiting.insert(key.clone())
                    {
                        if let Some(module) = self.functions.get(&key).copied()
                            && let FlowIrModule::Function { params, steps, .. } = module
                        {
                            let mut function_symbols = symbols.clone();
                            bind_approval_function_params(
                                params,
                                args,
                                symbols,
                                outputs,
                                &mut function_symbols,
                            );
                            let mut function_outputs = HashMap::new();
                            let function_return = self.collect_steps(
                                steps,
                                &format!("{step_path}/function/{key}"),
                                &mut function_symbols,
                                &mut function_outputs,
                                branches,
                                visiting,
                            );
                            call_output.merge(&function_return);
                        }
                        visiting.remove(&key);
                    }
                    outputs.insert(normalize(id), call_output);
                }
                FlowIrStep::If {
                    condition,
                    then_steps,
                    else_steps,
                    ..
                } => {
                    let if_index = self.flow.ifs.len();
                    self.flow.ifs.push(ApprovalIfEvidence {
                        index: if_index,
                        path: step_path.clone(),
                        condition: approval_value_evidence(condition, symbols, outputs),
                        condition_has_runtime_input: flow_ir_value_has_runtime_input(condition),
                    });
                    self.flow.next_sequence += 1;

                    let mut then_symbols = symbols.clone();
                    let mut then_outputs = outputs.clone();
                    branches.push(ApprovalBranchMembership {
                        if_index,
                        side: ApprovalBranchSide::Then,
                    });
                    self.collect_steps(
                        then_steps,
                        &format!("{step_path}/then_steps"),
                        &mut then_symbols,
                        &mut then_outputs,
                        branches,
                        visiting,
                    );
                    branches.pop();

                    let mut else_symbols = symbols.clone();
                    let mut else_outputs = outputs.clone();
                    branches.push(ApprovalBranchMembership {
                        if_index,
                        side: ApprovalBranchSide::Else,
                    });
                    self.collect_steps(
                        else_steps,
                        &format!("{step_path}/else_steps"),
                        &mut else_symbols,
                        &mut else_outputs,
                        branches,
                        visiting,
                    );
                    branches.pop();
                }
                FlowIrStep::ForEach {
                    array,
                    item,
                    index,
                    steps,
                    ..
                } => {
                    let mut loop_symbols = symbols.clone();
                    let mut item_evidence = approval_value_evidence(array, symbols, outputs);
                    item_evidence.add_dynamic(item);
                    loop_symbols.insert(normalize(item), item_evidence);
                    if let Some(index) = index {
                        let mut index_evidence = ApprovalValueEvidence::default();
                        index_evidence.add_dynamic(index);
                        loop_symbols.insert(normalize(index), index_evidence);
                    }
                    let mut loop_outputs = outputs.clone();
                    self.collect_steps(
                        steps,
                        &format!("{step_path}/loop_steps"),
                        &mut loop_symbols,
                        &mut loop_outputs,
                        branches,
                        visiting,
                    );
                }
                FlowIrStep::Assign { target, value } => {
                    let mut value = approval_value_evidence(value, symbols, outputs);
                    value.add_dynamic(target);
                    symbols.insert(normalize(target), value);
                }
                FlowIrStep::Return { values } => {
                    for value in values {
                        returned.merge(&approval_value_evidence(value, symbols, outputs));
                    }
                    break;
                }
            }
        }
        returned
    }
}

fn collect_approval_flow_evidence(
    program: &FlowIrProgram,
    catalog_by_name: &HashMap<String, &NodeMetadata>,
) -> ApprovalFlowEvidence {
    let functions = program
        .modules
        .iter()
        .filter(|module| matches!(module, FlowIrModule::Function { .. }))
        .map(|module| (normalize(module.name()), module))
        .collect::<HashMap<_, _>>();
    let mut globals = HashMap::new();
    for variable in &program.variables {
        let mut evidence = variable
            .default
            .as_ref()
            .map(approval_literal_evidence)
            .unwrap_or_default();
        evidence.add_dynamic(&variable.name);
        globals.insert(normalize(&variable.name), evidence);
    }

    let mut collector = ApprovalEvidenceCollector {
        catalog_by_name,
        functions,
        flow: ApprovalFlowEvidence::default(),
        module_index: 0,
    };
    for (module_index, module) in program.modules.iter().enumerate() {
        let FlowIrModule::Event { params, steps, .. } = module else {
            continue;
        };
        collector.module_index = module_index;
        let mut symbols = globals.clone();
        seed_approval_params(params, &mut symbols);
        collector.collect_steps(
            steps,
            &format!("/modules/{module_index}"),
            &mut symbols,
            &mut HashMap::new(),
            &mut Vec::new(),
            &mut HashSet::new(),
        );
    }
    collector.flow
}

fn seed_approval_params(
    params: &[FlowIrParam],
    symbols: &mut HashMap<String, ApprovalValueEvidence>,
) {
    for param in params {
        let mut evidence = ApprovalValueEvidence::default();
        evidence.add_dynamic(&param.name);
        symbols.insert(normalize(&param.name), evidence);
    }
}

fn bind_approval_function_params(
    params: &[FlowIrParam],
    args: &[FlowIrArg],
    caller_symbols: &HashMap<String, ApprovalValueEvidence>,
    caller_outputs: &HashMap<String, ApprovalValueEvidence>,
    function_symbols: &mut HashMap<String, ApprovalValueEvidence>,
) {
    for param in params {
        let mut evidence = args
            .iter()
            .find(|arg| arg.pin.eq_ignore_ascii_case(&param.name))
            .map(|arg| approval_value_evidence(&arg.value, caller_symbols, caller_outputs))
            .unwrap_or_default();
        evidence.add_dynamic(&param.name);
        function_symbols.insert(normalize(&param.name), evidence);
    }
}

fn approval_args_evidence(
    args: &[FlowIrArg],
    symbols: &HashMap<String, ApprovalValueEvidence>,
    outputs: &HashMap<String, ApprovalValueEvidence>,
) -> ApprovalValueEvidence {
    let mut evidence = ApprovalValueEvidence::default();
    for arg in args {
        evidence.add_label(&arg.pin);
        evidence.merge(&approval_value_evidence(&arg.value, symbols, outputs));
    }
    evidence
}

fn approval_recipient_evidence(
    args: &[FlowIrArg],
    symbols: &HashMap<String, ApprovalValueEvidence>,
    outputs: &HashMap<String, ApprovalValueEvidence>,
) -> ApprovalValueEvidence {
    let mut evidence = ApprovalValueEvidence::default();
    for arg in args {
        if approval_recipient_pin(&arg.pin) {
            evidence.merge(&approval_value_evidence(&arg.value, symbols, outputs));
        }
    }
    evidence
}

fn approval_recipient_pin(pin: &str) -> bool {
    let pin = normalize(pin);
    !pin.contains("replyto")
        && !pin.contains("sender")
        && !pin.starts_with("from")
        && matches!(
            pin.as_str(),
            "to" | "cc"
                | "bcc"
                | "recipient"
                | "recipients"
                | "recipientemail"
                | "recipientemails"
                | "emailaddress"
                | "emailaddresses"
                | "addresses"
        )
}

fn approval_value_evidence(
    value: &FlowIrValue,
    symbols: &HashMap<String, ApprovalValueEvidence>,
    outputs: &HashMap<String, ApprovalValueEvidence>,
) -> ApprovalValueEvidence {
    match value {
        FlowIrValue::Literal { value } => approval_literal_evidence(value),
        FlowIrValue::Ref { name } => {
            let mut evidence = symbols.get(&normalize(name)).cloned().unwrap_or_default();
            evidence.add_dynamic(name);
            evidence
        }
        FlowIrValue::Output { step, pin, .. } => {
            // Field access on a bound function parameter, loop item, or local alias arrives here
            // with the symbol name in `step`; fall back to its bound evidence so values threaded
            // through helper layers keep their caller-side provenance.
            let mut evidence = outputs
                .get(&normalize(step))
                .or_else(|| symbols.get(&normalize(step)))
                .cloned()
                .unwrap_or_default();
            evidence.add_dynamic(step);
            evidence.add_dynamic(pin);
            evidence
        }
        FlowIrValue::List { items } => {
            let mut evidence = ApprovalValueEvidence::default();
            for item in items {
                evidence.merge(&approval_value_evidence(item, symbols, outputs));
            }
            evidence
        }
        FlowIrValue::Object { fields } => {
            let mut evidence = ApprovalValueEvidence::default();
            for field in fields {
                evidence.add_label(&field.key);
                evidence.merge(&approval_value_evidence(&field.value, symbols, outputs));
            }
            evidence
        }
        FlowIrValue::FunctionRefs { functions } => {
            let mut evidence = ApprovalValueEvidence::default();
            for function in functions {
                evidence.add_label(function);
            }
            evidence
        }
    }
}

fn approval_literal_evidence(literal: &FlowIrLiteral) -> ApprovalValueEvidence {
    let mut evidence = ApprovalValueEvidence::default();
    match literal {
        FlowIrLiteral::String(value) => {
            evidence.add_label(value);
            evidence
                .literal_emails
                .extend(extract_email_literals(value));
        }
        FlowIrLiteral::Json(value) => add_approval_json_evidence(value, &mut evidence),
        FlowIrLiteral::Integer(_)
        | FlowIrLiteral::Float(_)
        | FlowIrLiteral::Boolean(_)
        | FlowIrLiteral::Null => {}
    }
    evidence
}

fn add_approval_json_evidence(value: &serde_json::Value, evidence: &mut ApprovalValueEvidence) {
    match value {
        serde_json::Value::String(value) => {
            evidence.add_label(value);
            evidence
                .literal_emails
                .extend(extract_email_literals(value));
        }
        serde_json::Value::Array(values) => {
            for value in values {
                add_approval_json_evidence(value, evidence);
            }
        }
        serde_json::Value::Object(fields) => {
            for (key, value) in fields {
                evidence.add_label(key);
                add_approval_json_evidence(value, evidence);
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
    }
}

fn ui_approval_loop_diagnostics(
    contract: &RequestApprovalLoopContract,
    program: &FlowIrProgram,
    catalog_by_name: &HashMap<String, &NodeMetadata>,
) -> Vec<FlowIrDiagnostic> {
    let evidence = collect_approval_flow_evidence(program, catalog_by_name);
    let event_params = program
        .modules
        .iter()
        .enumerate()
        .filter_map(|(module_index, module)| {
            let FlowIrModule::Event {
                node_type, params, ..
            } = module
            else {
                return None;
            };
            let node_type = normalize(node_type);
            if !node_type.contains("eventsgeneric") && !node_type.contains("eventgeneric") {
                return None;
            }
            Some((
                module_index,
                params
                    .iter()
                    .map(|param| normalize(&param.name))
                    .collect::<HashSet<_>>(),
            ))
        })
        .collect::<Vec<_>>();
    let approve_entries = event_params
        .iter()
        .filter(|(_, params)| {
            ui_params_have_ticket(params)
                && ui_params_have_draft_reply(params)
                && !ui_params_have_revision_feedback(params)
        })
        .map(|(module_index, _)| *module_index)
        .collect::<HashSet<_>>();
    let revise_entries = event_params
        .iter()
        .filter(|(_, params)| {
            ui_params_have_ticket(params) && ui_params_have_revision_feedback(params)
        })
        .map(|(module_index, _)| *module_index)
        .collect::<HashSet<_>>();

    let mut diagnostics = Vec::new();
    if approve_entries.is_empty() || revise_entries.is_empty() {
        diagnostics.push(approval_diagnostic(
            "IR_REQUEST_APPROVAL_UI_ACTIONS_MISSING",
            "/modules",
            "The requested approval page needs distinct typed approve and revise action entries.",
            "one eventsGeneric entry with ticketId + draftReply and one with ticketId + revisionFeedback",
            &format!(
                "approve entries {}, revise entries {}",
                approve_entries.len(),
                revise_entries.len()
            ),
            "Create separate approve and revise eventsGeneric handlers. The page action selects the handler; do not invent reviewerEmail or decision payload fields.",
        ));
        return diagnostics;
    }

    let expected_reviewers = contract
        .reviewer_emails
        .iter()
        .map(|email| email.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    let inferred_reviewers = evidence
        .nodes
        .iter()
        .filter(|node| revise_entries.contains(&node.module_index) && node.is_email_send)
        .flat_map(|node| node.recipient.literal_emails.iter().cloned())
        .collect::<HashSet<_>>();
    let reviewer_addresses = if expected_reviewers.is_empty() {
        inferred_reviewers
    } else {
        expected_reviewers
    };
    let approve_nodes = evidence
        .nodes
        .iter()
        .filter(|node| approve_entries.contains(&node.module_index))
        .collect::<Vec<_>>();
    let approve_customer_sends = approve_nodes
        .iter()
        .filter(|node| node.is_email_send && !approval_node_is_reviewer(node, &reviewer_addresses))
        .count();
    let approve_generations = approve_nodes
        .iter()
        .filter(|node| node.is_draft_generation)
        .count();
    let approve_reviewer_sends = approve_nodes
        .iter()
        .filter(|node| approval_node_is_reviewer(node, &reviewer_addresses))
        .count();
    if approve_customer_sends == 0 || approve_generations > 0 || approve_reviewer_sends > 0 {
        diagnostics.push(approval_diagnostic(
            "IR_REQUEST_APPROVAL_UI_APPROVE_UNSAFE",
            "/modules",
            "The approve action must send the submitted draft to the customer without regenerating it or re-routing it to the reviewer.",
            "a customer email send in the approve entry, with no model generation and no reviewer re-send",
            &format!(
                "customer sends {approve_customer_sends}, generations {approve_generations}, reviewer sends {approve_reviewer_sends}"
            ),
            "Keep customer delivery only in the ticketId + draftReply handler. A malformed approval must fail instead of falling into the revise path.",
        ));
    }

    let revise_nodes = evidence
        .nodes
        .iter()
        .filter(|node| revise_entries.contains(&node.module_index))
        .collect::<Vec<_>>();
    let revise_generations = revise_nodes
        .iter()
        .copied()
        .filter(|node| node.is_draft_generation && approval_has_feedback(&node.args))
        .collect::<Vec<_>>();
    let revise_reviewer_sends = revise_nodes
        .iter()
        .copied()
        .filter(|node| {
            approval_node_is_reviewer(node, &reviewer_addresses)
                && revise_generations
                    .iter()
                    .any(|generation| generation.sequence < node.sequence)
        })
        .collect::<Vec<_>>();
    let revise_customer_sends = revise_nodes
        .iter()
        .filter(|node| node.is_email_send && !approval_node_is_reviewer(node, &reviewer_addresses))
        .count();
    if revise_generations.is_empty()
        || revise_reviewer_sends.is_empty()
        || revise_customer_sends > 0
    {
        diagnostics.push(approval_diagnostic(
            "IR_REQUEST_APPROVAL_UI_REVISE_INCOMPLETE",
            "/modules",
            "The revise action must consume reviewer feedback, regenerate the draft, and then send the new draft back to the exact reviewer without emailing the customer.",
            "feedback-driven generation followed by an exact-reviewer email in the revise entry and no customer send",
            &format!(
                "feedback generations {}, reviewer re-sends {}, customer sends {revise_customer_sends}",
                revise_generations.len(),
                revise_reviewer_sends.len()
            ),
            "Use the ticketId + revisionFeedback handler exclusively for revision, persist the new revision, then re-send its approval link to the same reviewer.",
        ));
    }

    let initial_reviewer_sends = evidence
        .nodes
        .iter()
        .filter(|node| {
            !revise_entries.contains(&node.module_index)
                && !approve_entries.contains(&node.module_index)
                && approval_node_is_reviewer(node, &reviewer_addresses)
        })
        .collect::<Vec<_>>();
    if reviewer_addresses.is_empty()
        || initial_reviewer_sends.is_empty()
        || revise_reviewer_sends.is_empty()
    {
        diagnostics.push(approval_diagnostic(
            "IR_REQUEST_APPROVAL_REVIEWER_MISMATCH",
            "/modules",
            "The UI approval loop does not send both the initial and revised draft to the same exact reviewer.",
            &if contract.reviewer_emails.is_empty() {
                "one stable literal reviewer recipient used by the initial request and revise re-send".to_string()
            } else {
                format!("literal reviewer recipient [{}]", sorted_owned_terms(&reviewer_addresses))
            },
            &format!(
                "initial reviewer sends {}, revise reviewer sends {}",
                initial_reviewer_sends.len(),
                revise_reviewer_sends.len()
            ),
            "Use the reviewer address from the immutable request in the initial approval email and every revise re-send; do not accept reviewerEmail from the page payload.",
        ));
    }

    let initial_ticket_correlation = initial_reviewer_sends
        .iter()
        .any(|node| approval_has_ticket(&node.args));
    let revise_ticket_correlation = revise_reviewer_sends
        .iter()
        .any(|node| approval_has_ticket(&node.args));
    if !initial_ticket_correlation || !revise_ticket_correlation {
        diagnostics.push(approval_diagnostic(
            "IR_REQUEST_APPROVAL_CORRELATION_MISSING",
            "/modules",
            "The initial and revised approval notifications must carry the runtime ticket identity used by the page actions.",
            "dynamic ticketId evidence in the initial reviewer request and revise re-send",
            &format!(
                "initial ticket correlation {initial_ticket_correlation}, revise ticket correlation {revise_ticket_correlation}"
            ),
            "Thread the same runtime ticketId through persistence, approval links, approve/revise handlers, and reviewer re-notifications.",
        ));
    }

    diagnostics
}

fn ui_params_have_ticket(params: &HashSet<String>) -> bool {
    params.iter().any(|param| {
        param == "ticketid" || param == "caseid" || param == "supportid" || param == "correlationid"
    })
}

fn ui_params_have_draft_reply(params: &HashSet<String>) -> bool {
    params.iter().any(|param| {
        param == "draftreply"
            || param == "editeddraft"
            || param == "replytext"
            || param == "approvedreply"
    })
}

fn ui_params_have_revision_feedback(params: &HashSet<String>) -> bool {
    params.iter().any(|param| {
        param == "revisionfeedback"
            || param == "reviewerfeedback"
            || param == "changefeedback"
            || param == "reviewcomment"
    })
}

fn approval_loop_diagnostics(
    contract: &RequestApprovalLoopContract,
    program: &FlowIrProgram,
    catalog_by_name: &HashMap<String, &NodeMetadata>,
) -> Vec<FlowIrDiagnostic> {
    let evidence = collect_approval_flow_evidence(program, catalog_by_name);
    let email_sends = evidence
        .nodes
        .iter()
        .filter(|node| node.is_email_send)
        .collect::<Vec<_>>();
    let actual_recipient_emails = email_sends
        .iter()
        .flat_map(|node| node.recipient.literal_emails.iter().cloned())
        .collect::<HashSet<_>>();
    if evidence.ifs.is_empty() {
        return vec![approval_diagnostic(
            "IR_REQUEST_APPROVAL_BRANCH_MISSING",
            "/modules",
            "The reachable workflow has no conditional branch implementing the requested human approval decision.",
            "a reachable non-literal approval branch with customer send only in the approved path and regeneration plus reviewer re-send in the change path",
            "no reachable conditional approval branch",
            "Add an `if` approval decision in FlowScript; a flat IMAP/model/SMTP sequence cannot satisfy an approval loop.",
        )];
    }

    let expected_reviewers = contract
        .reviewer_emails
        .iter()
        .map(|email| email.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    let mut candidates = evidence
        .ifs
        .iter()
        .map(|branch| approval_candidate(branch, &evidence.nodes, &expected_reviewers))
        .collect::<Vec<_>>();
    candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.score()));
    let candidate = &candidates[0];
    let mut diagnostics = Vec::new();

    let expected_identity_present = if expected_reviewers.is_empty() {
        !candidate.reviewer_addresses.is_empty()
    } else {
        expected_reviewers
            .iter()
            .any(|email| actual_recipient_emails.contains(email))
    };
    if !expected_identity_present || candidate.initial_reviewer_sends.is_empty() {
        diagnostics.push(approval_diagnostic(
            "IR_REQUEST_APPROVAL_REVIEWER_MISMATCH",
            &candidate.branch.path,
            "The approval flow does not send an initial review request to the exact host-derived reviewer literal.",
            &if expected_reviewers.is_empty() {
                "one stable literal reviewer recipient reused by the initial request and else re-send".to_string()
            } else {
                format!("literal reviewer recipient [{}]", sorted_owned_terms(&expected_reviewers))
            },
            &format!(
                "reachable literal email recipients [{}]",
                sorted_owned_terms(&actual_recipient_emails)
            ),
            "Use the exact reviewer address from the user request as a recipient literal (or a variable with that literal default) in both review sends.",
        ));
    }

    if !candidate.condition_valid {
        diagnostics.push(approval_diagnostic(
            "IR_REQUEST_APPROVAL_CONDITION_UNCORRELATED",
            &format!("{}/condition", candidate.branch.path),
            "The approval branch condition is literal or is not correlated to a reviewer decision, ticket, and draft version.",
            "a runtime decision value carrying dynamic approval, ticket-id, and draft-version evidence",
            &format!(
                "condition dynamic keys [{}]",
                sorted_owned_terms(&candidate.branch.condition.dynamic_keys)
            ),
            "Parse the reviewer response, correlate it to the open ticket and current draft version, and branch on that runtime decision output.",
        ));
    }

    if !candidate.condition_reviewer_valid {
        diagnostics.push(approval_diagnostic(
            "IR_REQUEST_APPROVAL_SENDER_UNVERIFIED",
            &format!("{}/condition", candidate.branch.path),
            "The approval decision can be reached without proving that the inbound sender equals the exact reviewer literal.",
            &format!(
                "condition dataflow containing reviewer literal [{}] plus the runtime inbound sender",
                sorted_owned_terms(&candidate.reviewer_addresses)
            ),
            &format!(
                "condition literal emails [{}]",
                sorted_owned_terms(&candidate.branch.condition.literal_emails)
            ),
            "Validate the inbound sender against the exact host-derived reviewer address and branch on that validation output; outbound recipient checks alone are insufficient.",
        ));
    }

    if candidate.customer_then_sends.is_empty() || !candidate.unguarded_customer_sends.is_empty() {
        diagnostics.push(approval_diagnostic(
            "IR_REQUEST_APPROVAL_CUSTOMER_SEND_UNGUARDED",
            &candidate.branch.path,
            "Customer-facing email is missing from the approved branch or another non-reviewer send is reachable outside it.",
            "at least one customer send in the approved branch and no non-reviewer email send outside that branch",
            &format!(
                "then customer sends {}, unguarded non-reviewer sends {}",
                candidate.customer_then_sends.len(),
                candidate.unguarded_customer_sends.len()
            ),
            "Move every customer send into the correlated approved branch; reviewer requests belong before the decision or in the change branch.",
        ));
    }

    if candidate.else_generations.is_empty()
        || candidate.else_reviewer_sends_after_generation.is_empty()
    {
        diagnostics.push(approval_diagnostic(
            "IR_REQUEST_APPROVAL_REASK_INCOMPLETE",
            &format!("{}/else_steps", candidate.branch.path),
            "The rejection/change branch does not regenerate the draft from dynamic reviewer feedback and then re-send it to the same literal reviewer.",
            "a reachable model generation consuming dynamic reviewer feedback, followed by an exact-reviewer email send in the change branch",
            &format!(
                "else generations {}, reviewer re-sends after generation {}",
                candidate.else_generations.len(),
                candidate.else_reviewer_sends_after_generation.len()
            ),
            "In the change branch, generate the revised draft first and then send a new reviewer request to the exact reviewer literal.",
        ));
    }

    if !candidate.correlation_valid {
        diagnostics.push(approval_diagnostic(
            "IR_REQUEST_APPROVAL_CORRELATION_MISSING",
            &candidate.branch.path,
            "Reviewer request/re-send and the decision condition do not all carry dynamic ticket-id and draft-version values.",
            "dynamic ticket and version values in the initial reviewer request, change-branch re-send, and approval condition",
            "one or more correlation dimensions are absent or literal-only",
            "Wire the same ticket identity and the current draft version through review subject/body values, response parsing, and the re-ask; placeholder text alone does not count.",
        ));
    }
    diagnostics
}

struct ApprovalCandidate<'a> {
    branch: &'a ApprovalIfEvidence,
    reviewer_addresses: HashSet<String>,
    initial_reviewer_sends: Vec<&'a ApprovalNodeEvidence>,
    customer_then_sends: Vec<&'a ApprovalNodeEvidence>,
    unguarded_customer_sends: Vec<&'a ApprovalNodeEvidence>,
    else_generations: Vec<&'a ApprovalNodeEvidence>,
    else_reviewer_sends_after_generation: Vec<&'a ApprovalNodeEvidence>,
    condition_valid: bool,
    condition_reviewer_valid: bool,
    correlation_valid: bool,
}

impl ApprovalCandidate<'_> {
    fn score(&self) -> usize {
        usize::from(!self.initial_reviewer_sends.is_empty())
            + usize::from(!self.customer_then_sends.is_empty())
            + usize::from(self.unguarded_customer_sends.is_empty())
            + usize::from(!self.else_generations.is_empty())
            + usize::from(!self.else_reviewer_sends_after_generation.is_empty())
            + usize::from(self.condition_valid)
            + usize::from(self.condition_reviewer_valid)
            + usize::from(self.correlation_valid)
    }
}

fn approval_candidate<'a>(
    branch: &'a ApprovalIfEvidence,
    nodes: &'a [ApprovalNodeEvidence],
    expected_reviewers: &HashSet<String>,
) -> ApprovalCandidate<'a> {
    let email_sends = nodes
        .iter()
        .filter(|node| node.is_email_send)
        .collect::<Vec<_>>();
    let reviewer_addresses = if expected_reviewers.is_empty() {
        let outside = email_sends
            .iter()
            .filter(|node| {
                !under_approval_branch(node, branch.index, ApprovalBranchSide::Then)
                    && !under_approval_branch(node, branch.index, ApprovalBranchSide::Else)
            })
            .flat_map(|node| node.recipient.literal_emails.iter().cloned())
            .collect::<HashSet<_>>();
        let in_else = email_sends
            .iter()
            .filter(|node| under_approval_branch(node, branch.index, ApprovalBranchSide::Else))
            .flat_map(|node| node.recipient.literal_emails.iter().cloned())
            .collect::<HashSet<_>>();
        outside.intersection(&in_else).cloned().collect()
    } else {
        expected_reviewers.clone()
    };
    let reviewer_sends = email_sends
        .iter()
        .copied()
        .filter(|node| approval_node_is_reviewer(node, &reviewer_addresses))
        .collect::<Vec<_>>();
    let initial_reviewer_sends = reviewer_sends
        .iter()
        .copied()
        .filter(|node| {
            !under_approval_branch(node, branch.index, ApprovalBranchSide::Then)
                && !under_approval_branch(node, branch.index, ApprovalBranchSide::Else)
        })
        .collect::<Vec<_>>();
    let non_reviewer_sends = email_sends
        .iter()
        .copied()
        .filter(|node| !approval_node_is_reviewer(node, &reviewer_addresses))
        .collect::<Vec<_>>();
    let customer_then_sends = non_reviewer_sends
        .iter()
        .copied()
        .filter(|node| under_approval_branch(node, branch.index, ApprovalBranchSide::Then))
        .collect::<Vec<_>>();
    let unguarded_customer_sends = non_reviewer_sends
        .iter()
        .copied()
        .filter(|node| !under_approval_branch(node, branch.index, ApprovalBranchSide::Then))
        .collect::<Vec<_>>();
    let else_generations = nodes
        .iter()
        .filter(|node| {
            node.is_draft_generation
                && under_approval_branch(node, branch.index, ApprovalBranchSide::Else)
                && approval_has_feedback(&node.args)
        })
        .collect::<Vec<_>>();
    let else_reviewer_sends_after_generation = reviewer_sends
        .iter()
        .copied()
        .filter(|send| {
            under_approval_branch(send, branch.index, ApprovalBranchSide::Else)
                && else_generations
                    .iter()
                    .any(|generation| generation.sequence < send.sequence)
        })
        .collect::<Vec<_>>();
    let condition_valid = approval_condition_valid(branch);
    let condition_reviewer_valid = !reviewer_addresses.is_empty()
        && branch
            .condition
            .literal_emails
            .iter()
            .any(|email| reviewer_addresses.contains(email))
        && approval_has_inbound_sender(&branch.condition);
    let correlation_valid = approval_has_ticket(&branch.condition)
        && approval_has_version(&branch.condition)
        && initial_reviewer_sends.iter().any(|initial| {
            approval_has_ticket(&initial.args)
                && approval_has_version(&initial.args)
                && else_reviewer_sends_after_generation.iter().any(|reask| {
                    approval_has_ticket(&reask.args) && approval_has_version(&reask.args)
                })
        });
    ApprovalCandidate {
        branch,
        reviewer_addresses,
        initial_reviewer_sends,
        customer_then_sends,
        unguarded_customer_sends,
        else_generations,
        else_reviewer_sends_after_generation,
        condition_valid,
        condition_reviewer_valid,
        correlation_valid,
    }
}

fn approval_node_is_reviewer(
    node: &ApprovalNodeEvidence,
    reviewer_addresses: &HashSet<String>,
) -> bool {
    node.recipient
        .literal_emails
        .iter()
        .any(|email| reviewer_addresses.contains(email))
        && node
            .recipient
            .literal_emails
            .iter()
            .all(|email| reviewer_addresses.contains(email))
        && !approval_has_customer_recipient(&node.recipient)
}

fn under_approval_branch(
    node: &ApprovalNodeEvidence,
    if_index: usize,
    side: ApprovalBranchSide,
) -> bool {
    node.branches
        .contains(&ApprovalBranchMembership { if_index, side })
}

fn approval_has_customer_recipient(evidence: &ApprovalValueEvidence) -> bool {
    evidence.dynamic_keys.iter().any(|key| {
        key.contains("customer")
            || key.contains("requester")
            || key.contains("kunde")
            || key.contains("anfragend")
    })
}

fn approval_condition_valid(branch: &ApprovalIfEvidence) -> bool {
    branch.condition_has_runtime_input
        && approval_has_decision(&branch.condition)
        && approval_has_ticket(&branch.condition)
        && approval_has_version(&branch.condition)
}

/// Whether any runtime ref/output flows into this value. A comparison or boolean expression over
/// runtime values (projected as an [`FlowIrValue::Object`]) is a runtime decision; a value built
/// purely from literals is not.
fn flow_ir_value_has_runtime_input(value: &FlowIrValue) -> bool {
    match value {
        FlowIrValue::Ref { .. } | FlowIrValue::Output { .. } => true,
        FlowIrValue::List { items } => items.iter().any(flow_ir_value_has_runtime_input),
        FlowIrValue::Object { fields } => fields
            .iter()
            .any(|field| flow_ir_value_has_runtime_input(&field.value)),
        FlowIrValue::Literal { .. } | FlowIrValue::FunctionRefs { .. } => false,
    }
}

fn approval_has_decision(evidence: &ApprovalValueEvidence) -> bool {
    evidence.terms.iter().any(|term| {
        matches!(
            term.as_str(),
            "approve"
                | "approved"
                | "approval"
                | "decision"
                | "reviewer"
                | "review"
                | "change"
                | "reject"
                | "rejected"
                | "confirm"
                | "confirmed"
                | "freigabe"
                | "freigegeben"
                | "bestätigung"
                | "bestaetigung"
                | "bestätigt"
                | "bestaetigt"
        )
    })
}

fn approval_has_ticket(evidence: &ApprovalValueEvidence) -> bool {
    evidence.dynamic_keys.iter().any(|key| {
        key.contains("ticket")
            || key.contains("caseid")
            || key.contains("supportid")
            || key.contains("correlationid")
    })
}

fn approval_has_version(evidence: &ApprovalValueEvidence) -> bool {
    evidence
        .dynamic_keys
        .iter()
        .any(|key| key.contains("version") || key.contains("revision"))
}

fn approval_has_feedback(evidence: &ApprovalValueEvidence) -> bool {
    evidence.dynamic_keys.iter().any(|key| {
        key.contains("feedback")
            || key.contains("change")
            || key.contains("revisionrequest")
            || key.contains("reviewcomment")
            || key.contains("anpassung")
            || key.contains("änderung")
            || key.contains("aenderung")
    })
}

fn approval_has_inbound_sender(evidence: &ApprovalValueEvidence) -> bool {
    evidence.dynamic_keys.iter().any(|key| {
        key.contains("incomingsender")
            || key.contains("reviewersender")
            || key.contains("senderemail")
            || key.contains("fromemail")
            || key == "sender"
            || key == "from"
    })
}

fn approval_diagnostic(
    code: &str,
    path: &str,
    message: &str,
    expected: &str,
    actual: &str,
    fix: &str,
) -> FlowIrDiagnostic {
    FlowIrDiagnostic {
        code: code.to_string(),
        phase: "validate".to_string(),
        path: path.to_string(),
        scope: Some("human approval loop".to_string()),
        message: message.to_string(),
        expected: Some(expected.to_string()),
        actual: Some(actual.to_string()),
        declaration: None,
        pin: None,
        fix: Some(fix.to_string()),
        caused_by: Vec::new(),
    }
}

fn assign_acceptance_occurrence(
    criterion_index: usize,
    candidate_occurrences: &[Vec<usize>],
    occurrence_owner: &mut [Option<usize>],
    visited: &mut [bool],
) -> bool {
    for &occurrence_index in &candidate_occurrences[criterion_index] {
        if visited[occurrence_index] {
            continue;
        }
        visited[occurrence_index] = true;
        let available = match occurrence_owner[occurrence_index] {
            None => true,
            Some(owner) => assign_acceptance_occurrence(
                owner,
                candidate_occurrences,
                occurrence_owner,
                visited,
            ),
        };
        if available {
            occurrence_owner[occurrence_index] = Some(criterion_index);
            return true;
        }
    }
    false
}

fn catalog_acceptance_semantics(
    metadata: &NodeMetadata,
) -> (HashSet<&'static str>, HashSet<String>) {
    let values = std::iter::once(metadata.name.as_str())
        .chain(std::iter::once(metadata.friendly_name.as_str()))
        .chain(std::iter::once(metadata.description.as_str()))
        .chain(metadata.category.iter().map(String::as_str))
        .chain(metadata.capability_tags.iter().map(String::as_str));
    let tokens = values.flat_map(semantic_tokens).collect::<Vec<_>>();
    let mut actions = tokens
        .iter()
        .filter_map(|token| canonical_action(token))
        .collect::<HashSet<_>>();
    let mut subjects = salient_subject_terms(&tokens)
        .into_iter()
        .collect::<HashSet<_>>();
    let normalized_name = normalize(&metadata.name);
    if normalized_name.starts_with("events") {
        actions.insert("trigger");
    }
    if normalized_name == "eventssimple" {
        // FlowLike's host registers external cron/timer events against the generic simple entry;
        // the board must not invent a cron catalog node merely to express that scheduling intent.
        subjects.insert("schedule".to_string());
    } else if normalized_name.contains("cron")
        || normalized_name.contains("scheduler")
        || normalized_name.contains("schedule")
    {
        subjects.insert("cron_catalog".to_string());
    }
    if normalized_name.contains("generative") || normalized_name.contains("llm") {
        actions.insert("generate");
    }
    if normalized_name.contains("insert") || normalized_name.contains("upsert") {
        actions.insert("write");
    }
    (actions, subjects)
}

fn acceptance_action_covered(action: &str, actual: &HashSet<&str>) -> bool {
    actual.contains(action)
        || match action {
            "create" => actual.iter().any(|candidate| {
                matches!(*candidate, "generate" | "write" | "transform" | "trigger")
            }),
            "change" | "update" => actual
                .iter()
                .any(|candidate| matches!(*candidate, "generate" | "transform" | "write")),
            "read" | "receive" => actual
                .iter()
                .any(|candidate| matches!(*candidate, "read" | "receive")),
            "send" | "call" => actual
                .iter()
                .any(|candidate| matches!(*candidate, "send" | "call" | "publish")),
            "approve" | "reject" => actual
                .iter()
                .any(|candidate| matches!(*candidate, "branch" | "validate")),
            "retry" => actual
                .iter()
                .any(|candidate| matches!(*candidate, "iterate" | "branch")),
            "schedule" => actual.contains("trigger"),
            _ => false,
        }
}

fn sorted_terms(values: &HashSet<&str>) -> String {
    let mut values = values.iter().copied().collect::<Vec<_>>();
    values.sort_unstable();
    values.join(", ")
}

fn sorted_owned_terms(values: &HashSet<String>) -> String {
    let mut values = values.iter().map(String::as_str).collect::<Vec<_>>();
    values.sort_unstable();
    values.join(", ")
}

fn bounded_summary(value: &str) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= MAX_FLOW_IR_ACCEPTANCE_SUMMARY_CHARS {
        return collapsed;
    }
    // Cut on a word boundary and mark the elision so a truncated scope never ends mid-word.
    let budget = MAX_FLOW_IR_ACCEPTANCE_SUMMARY_CHARS.saturating_sub(2);
    let mut bounded = String::new();
    for word in collapsed.split(' ') {
        let separator = usize::from(!bounded.is_empty());
        if bounded.chars().count() + separator + word.chars().count() > budget {
            break;
        }
        if separator == 1 {
            bounded.push(' ');
        }
        bounded.push_str(word);
    }
    if bounded.is_empty() {
        bounded = collapsed.chars().take(budget).collect();
    }
    bounded.push_str(" …");
    bounded
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn normalized_name_set(values: &[String]) -> HashSet<String> {
    values
        .iter()
        .map(|value| normalize(value))
        .filter(|value| !value.is_empty())
        .collect()
}

fn capability_plan_has_oversized_names(request: &FlowCapabilityPlanRequest) -> bool {
    request.requirements.iter().any(|requirement| {
        requirement.id.len() > MAX_FLOW_IR_AUTHORED_NAME_BYTES
            || requirement
                .exact_node_type
                .as_ref()
                .is_some_and(|name| name.len() > MAX_FLOW_IR_AUTHORED_NAME_BYTES)
            || requirement
                .inputs
                .iter()
                .chain(&requirement.outputs)
                .flat_map(|pin| &pin.names)
                .any(|name| name.len() > MAX_FLOW_IR_AUTHORED_NAME_BYTES)
    }) || request
        .modules
        .iter()
        .any(|module| module.name.len() > MAX_FLOW_IR_AUTHORED_NAME_BYTES)
}

fn capability_request_limit_error(
    request: &FlowCapabilityPlanRequest,
) -> Option<(&'static str, String)> {
    if serde_json::to_vec(request)
        .map(|encoded| encoded.len() > MAX_FLOW_IR_CAPABILITY_PLAN_BYTES)
        .unwrap_or(true)
    {
        return Some((
            "IR_CAPABILITY_PLAN_SIZE_LIMIT_EXCEEDED",
            format!(
                "capability plan must be at most {MAX_FLOW_IR_CAPABILITY_PLAN_BYTES} serialized bytes"
            ),
        ));
    }
    if capability_plan_has_oversized_names(request) {
        return Some((
            "IR_AUTHORED_NAME_TOO_LONG",
            format!(
                "capability ids, node/pin names, and module names must be at most {MAX_FLOW_IR_AUTHORED_NAME_BYTES} bytes"
            ),
        ));
    }
    if request
        .requirements
        .iter()
        .any(|requirement| requirement.intent.len() > MAX_FLOW_IR_CAPABILITY_INTENT_BYTES)
    {
        return Some((
            "IR_CAPABILITY_INTENT_TOO_LONG",
            format!(
                "capability intent text must be at most {MAX_FLOW_IR_CAPABILITY_INTENT_BYTES} bytes"
            ),
        ));
    }
    if request.requirements.len() > MAX_FLOW_IR_CAPABILITY_REQUIREMENTS {
        return Some((
            "IR_CAPABILITY_REQUIREMENT_LIMIT_EXCEEDED",
            format!(
                "capability_plan may contain at most {MAX_FLOW_IR_CAPABILITY_REQUIREMENTS} requirements"
            ),
        ));
    }
    if request.requirements.iter().any(|requirement| {
        requirement.inputs.len() > MAX_FLOW_IR_PIN_REQUIREMENTS_PER_DIRECTION
            || requirement.outputs.len() > MAX_FLOW_IR_PIN_REQUIREMENTS_PER_DIRECTION
    }) {
        return Some((
            "IR_CAPABILITY_PIN_REQUIREMENT_LIMIT_EXCEEDED",
            format!(
                "each capability may contain at most {MAX_FLOW_IR_PIN_REQUIREMENTS_PER_DIRECTION} input and output pin requirements"
            ),
        ));
    }
    if request.requirements.iter().any(|requirement| {
        requirement
            .inputs
            .iter()
            .chain(&requirement.outputs)
            .any(|pin| pin.names.len() > MAX_FLOW_IR_PIN_ALIASES_PER_REQUIREMENT)
    }) {
        return Some((
            "IR_CAPABILITY_PIN_ALIAS_LIMIT_EXCEEDED",
            format!(
                "each pin requirement may contain at most {MAX_FLOW_IR_PIN_ALIASES_PER_REQUIREMENT} names"
            ),
        ));
    }
    if request.modules.len() > super::ir::MAX_FLOW_IR_MODULES {
        return Some((
            "IR_CAPABILITY_MODULE_LIMIT_EXCEEDED",
            format!(
                "capability_plan may estimate at most {} modules",
                super::ir::MAX_FLOW_IR_MODULES
            ),
        ));
    }
    if !request
        .requirements
        .iter()
        .any(|requirement| requirement.required)
    {
        return Some((
            "IR_CAPABILITY_PLAN_REQUIRED",
            "capability_plan must include at least one required catalog capability produced by plan_flow_ir"
                .to_string(),
        ));
    }
    None
}

fn removed_required_capabilities(
    current: &FlowCapabilityPlanRequest,
    candidate: &FlowCapabilityPlanRequest,
) -> Vec<String> {
    let mut removed = current
        .requirements
        .iter()
        .filter(|requirement| requirement.required)
        .filter(|requirement| {
            !candidate.requirements.iter().any(|replacement| {
                replacement.required
                    && normalize(&replacement.id) == normalize(&requirement.id)
                    && replacement
                        .exact_node_type
                        .as_ref()
                        .map(|name| normalize(name))
                        == requirement
                            .exact_node_type
                            .as_ref()
                            .map(|name| normalize(name))
                    && replacement.inputs == requirement.inputs
                    && replacement.outputs == requirement.outputs
            })
        })
        .map(|requirement| requirement.id.clone())
        .collect::<Vec<_>>();
    removed.sort();
    removed
}

fn expected_module_contract(
    names: &[String],
    capability_request: &FlowCapabilityPlanRequest,
) -> Result<HashMap<String, FlowModuleKind>, (&'static str, String)> {
    let mut estimates = HashMap::new();
    for estimate in &capability_request.modules {
        let key = normalize(&estimate.name);
        if key.is_empty() {
            return Err((
                "IR_MODULE_ESTIMATE_NAME_INVALID",
                "capability module estimates must have non-empty names".to_string(),
            ));
        }
        if estimates.insert(key.clone(), estimate.kind).is_some() {
            return Err((
                "IR_MODULE_ESTIMATE_DUPLICATE",
                format!("capability plan estimates module {key:?} more than once"),
            ));
        }
    }

    let mut expected = HashMap::new();
    for name in names {
        let key = normalize(name);
        if key.is_empty() {
            continue;
        }
        if expected.contains_key(&key) {
            return Err((
                "IR_EXPECTED_MODULE_DUPLICATE",
                format!("expected module {name:?} is listed more than once"),
            ));
        }
        let Some(kind) = estimates.get(&key).copied() else {
            return Err((
                "IR_EXPECTED_MODULE_ESTIMATE_MISSING",
                format!(
                    "expected module {name:?} must have exactly one capability_plan.modules estimate with its Function/Event kind"
                ),
            ));
        };
        expected.insert(key, kind);
    }
    Ok(expected)
}

fn expected_module_diagnostics(
    program: &FlowIrProgram,
    expected_modules: &HashMap<String, FlowModuleKind>,
) -> Vec<FlowIrDiagnostic> {
    program
        .modules
        .iter()
        .enumerate()
        .filter_map(|(index, module)| {
            let key = normalize(module.name());
            let expected = expected_modules.get(&key)?;
            let actual = match module {
                FlowIrModule::Function { .. } => FlowModuleKind::Function,
                FlowIrModule::Event { .. } => FlowModuleKind::Event,
            };
            (actual != *expected).then(|| {
                let mut diagnostic = FlowIrDiagnostic {
                    code: "IR_MODULE_KIND_MISMATCH".to_string(),
                    phase: "draft".to_string(),
                    path: format!("/modules/{index}/kind"),
                    scope: Some(module.name().to_string()),
                    message: format!(
                        "module {:?} is authored as {actual:?}, but its capability plan requires {expected:?}",
                        module.name()
                    ),
                    expected: Some(format!("{expected:?}")),
                    actual: Some(format!("{actual:?}")),
                    declaration: Some(module.name().to_string()),
                    pin: None,
                    fix: Some(
                        "use the planned Function/Event module kind; do not satisfy an Event requirement with a same-named Function"
                            .to_string(),
                    ),
                    caused_by: Vec::new(),
                };
                diagnostic.phase.make_ascii_lowercase();
                diagnostic
            })
        })
        .collect()
}

fn stored_draft_size(draft: &StoredDraft) -> usize {
    serde_json::to_vec(&(
        &draft.base_fingerprint,
        &draft.board_id,
        &draft.expected_modules,
        &draft.capability_request,
        &draft.capability_plan,
        &draft.request_acceptance_contract,
        draft.request_identity.as_str(),
        draft.mode,
        &draft.program,
        &draft.staged_evaluation,
        &draft.validated,
        &draft.best,
        &draft.pending_claim_id,
        &draft.pending_commands,
    ))
    .map(|encoded| encoded.len())
    .unwrap_or(MAX_FLOW_IR_DRAFT_STORE_BYTES.saturating_add(1))
}

fn missing_modules(draft: &StoredDraft) -> Vec<String> {
    missing_modules_for_program(&draft.expected_modules, &draft.program)
}

fn missing_modules_for_program(
    expected_modules: &HashMap<String, FlowModuleKind>,
    program: &FlowIrProgram,
) -> Vec<String> {
    let present = program
        .modules
        .iter()
        .map(|module| normalize(module.name()))
        .collect::<HashSet<_>>();
    let mut missing = expected_modules
        .keys()
        .filter(|name| !present.contains(*name))
        .cloned()
        .collect::<Vec<_>>();
    missing.sort();
    missing
}

fn removed_module_scope_items(
    current: &FlowIrProgram,
    candidate: &FlowIrProgram,
    normalized_name: &str,
) -> Vec<String> {
    let current_items = current
        .modules
        .iter()
        .find(|module| normalize(module.name()) == normalized_name)
        .map(module_scope_items)
        .unwrap_or_default();
    let candidate_items = candidate
        .modules
        .iter()
        .find(|module| normalize(module.name()) == normalized_name)
        .map(module_scope_items)
        .unwrap_or_default();
    let mut removed = current_items
        .into_iter()
        .filter_map(|(item, count)| {
            let missing = count.saturating_sub(candidate_items.get(&item).copied().unwrap_or(0));
            (missing > 0).then(|| {
                if missing == 1 {
                    item
                } else {
                    format!("{item} x{missing}")
                }
            })
        })
        .collect::<Vec<_>>();
    removed.sort();
    removed
}

fn module_scope_items(module: &FlowIrModule) -> HashMap<String, usize> {
    fn insert(items: &mut HashMap<String, usize>, item: String) {
        *items.entry(item).or_default() += 1;
    }

    fn visit_steps(steps: &[FlowIrStep], items: &mut HashMap<String, usize>) {
        for step in steps {
            match step {
                FlowIrStep::Node { id, exec_arms, .. } => {
                    insert(items, format!("node:{}", normalize(id)));
                    for arm in exec_arms {
                        visit_steps(&arm.steps, items);
                    }
                }
                FlowIrStep::CallFunction { id, .. } => {
                    insert(items, format!("call:{}", normalize(id)));
                }
                FlowIrStep::If {
                    id,
                    then_steps,
                    else_steps,
                    ..
                } => {
                    insert(items, format!("if:{}", normalize(id)));
                    visit_steps(then_steps, items);
                    visit_steps(else_steps, items);
                }
                FlowIrStep::ForEach { id, steps, .. } => {
                    insert(items, format!("for_each:{}", normalize(id)));
                    visit_steps(steps, items);
                }
                FlowIrStep::Assign { target, .. } => {
                    insert(items, format!("assign:{}", normalize(target)));
                }
                FlowIrStep::Return { .. } => insert(items, "return".to_string()),
            }
        }
    }

    let mut items = HashMap::new();
    match module {
        FlowIrModule::Function {
            params,
            returns,
            steps,
            ..
        } => {
            for param in params {
                insert(&mut items, format!("parameter:{}", normalize(&param.name)));
            }
            for return_param in returns {
                insert(
                    &mut items,
                    format!("return_parameter:{}", normalize(&return_param.name)),
                );
            }
            visit_steps(steps, &mut items);
        }
        FlowIrModule::Event { params, steps, .. } => {
            for param in params {
                insert(&mut items, format!("parameter:{}", normalize(&param.name)));
            }
            visit_steps(steps, &mut items);
        }
    }
    items
}

fn board_fingerprint(board: &Board) -> String {
    let source = board_to_flowscript(
        board,
        &RenderOptions {
            anchors: true,
            ..Default::default()
        },
    );
    // Deterministic FNV-1a is sufficient for optimistic concurrency; this is not a security token.
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in source.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

#[derive(Serialize)]
struct FlowScriptCatalogNodeContract<'a> {
    name: &'a str,
    friendly_name: &'a str,
    description: &'a str,
    category: &'a Option<String>,
    capability_tags: Vec<&'a str>,
    inputs: Vec<FlowScriptCatalogPinContract<'a>>,
    outputs: Vec<FlowScriptCatalogPinContract<'a>>,
    required_inputs: Vec<&'a str>,
}

#[derive(Serialize)]
struct FlowScriptCatalogPinContract<'a> {
    name: &'a str,
    data_type: &'a str,
    value_type: &'a str,
    default_value: &'a Option<String>,
    schema: &'a Option<String>,
    is_generic: bool,
    valid_values: &'a Option<Vec<String>>,
    enforce_schema: bool,
}

impl<'a> From<&'a PinMetadata> for FlowScriptCatalogPinContract<'a> {
    fn from(pin: &'a PinMetadata) -> Self {
        Self {
            name: &pin.name,
            data_type: &pin.data_type,
            value_type: &pin.value_type,
            default_value: &pin.default_value,
            schema: &pin.schema,
            is_generic: pin.is_generic,
            valid_values: &pin.valid_values,
            enforce_schema: pin.enforce_schema,
        }
    }
}

impl<'a> From<&'a NodeMetadata> for FlowScriptCatalogNodeContract<'a> {
    fn from(metadata: &'a NodeMetadata) -> Self {
        let mut required_inputs = metadata
            .required_inputs
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        required_inputs.sort_unstable();
        required_inputs.dedup();
        let mut capability_tags = metadata
            .capability_tags
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        capability_tags.sort_unstable();
        capability_tags.dedup();
        Self {
            name: &metadata.name,
            friendly_name: &metadata.friendly_name,
            description: &metadata.description,
            category: &metadata.category,
            capability_tags,
            inputs: metadata.inputs.iter().map(Into::into).collect(),
            outputs: metadata.outputs.iter().map(Into::into).collect(),
            required_inputs,
        }
    }
}

/// Hash the executable and host-acceptance catalog contract independently of provider iteration
/// order. Node descriptions, labels, categories, and capability tags are included because request
/// acceptance intentionally derives semantic coverage from them. Companion-search hints and pin
/// presentation text are excluded because they cannot change a checked command batch or its
/// acceptance result. Pin order remains significant because repeated same-name pins are
/// occurrence-addressed during reconciliation. Each node is serialized independently and sorted as
/// bytes, preserving duplicate declarations while making equivalent catalog permutations produce
/// the same fingerprint.
fn flowscript_catalog_fingerprint(catalog: &[NodeMetadata]) -> String {
    let mut contracts = catalog
        .iter()
        .map(|metadata| {
            serde_json::to_vec(&FlowScriptCatalogNodeContract::from(metadata))
                .expect("catalog contracts contain only deterministically serializable fields")
        })
        .collect::<Vec<_>>();
    contracts.sort_unstable();

    let mut hasher = blake3::Hasher::new();
    hasher.update(FLOWSCRIPT_CATALOG_FINGERPRINT_DOMAIN);
    hasher.update(&(contracts.len() as u64).to_le_bytes());
    for contract in contracts {
        hasher.update(&(contract.len() as u64).to_le_bytes());
        hasher.update(&contract);
    }
    format!("b3:{}", hasher.finalize().to_hex())
}

/// Scope represented by a typed draft when it is reconciled with the live board.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FlowIrDraftMode {
    /// Add the typed program while preserving every unrelated existing board node and variable.
    #[default]
    Additive,
    /// Treat the typed program as the replacement document. Commit still requires an exact
    /// per-entity allowlist for every existing node, variable, layer, or comment it would remove.
    Replace,
}

impl FlowIrDraftMode {
    fn reconcile_mode(self) -> ReconcileMode {
        match self {
            Self::Additive => ReconcileMode::Additive,
            Self::Replace => ReconcileMode::Replace,
        }
    }
}

/// Begin a retained code-first FlowScript session. The source is kept byte-for-byte so streamed
/// tool arguments and every subsequent response can be rendered as the same editable document.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WriteFlowScriptArgs {
    pub draft_id: String,
    #[serde(default)]
    pub replace_existing: bool,
    /// Defaults to additive. Use replace only for a complete board document whose exact removals
    /// the user has authorized at commit time.
    #[serde(default)]
    pub mode: FlowIrDraftMode,
    #[serde(alias = "flowscript", alias = "script", alias = "content")]
    pub source: String,
    /// Explicit gate for intentionally replacing a substantial retained application with a much
    /// smaller one. Ordinary in-place repairs never need this.
    #[serde(default)]
    pub allow_scope_reduction: bool,
}

/// Apply one deterministic textual repair to a retained source document. `old_text` must occur
/// exactly once; ambiguous or already-applied patches are rejected without changing the revision.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PatchFlowScriptArgs {
    pub draft_id: String,
    pub expected_revision: u64,
    #[serde(alias = "search")]
    pub old_text: String,
    #[serde(alias = "replacement")]
    pub new_text: String,
    #[serde(default)]
    pub allow_scope_reduction: bool,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckFlowScriptArgs {
    pub draft_id: String,
    pub expected_revision: u64,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommitFlowScriptArgs {
    pub draft_id: String,
    pub expected_revision: u64,
    /// Compatibility flag only. Replacement commits still enumerate every exact removal id.
    #[serde(default)]
    pub allow_deletions: bool,
    #[serde(default)]
    pub remove_node_ids: Vec<String>,
    #[serde(default)]
    pub remove_variable_ids: Vec<String>,
    #[serde(default)]
    pub remove_layer_ids: Vec<String>,
    #[serde(default)]
    pub remove_comment_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FlowScriptDraftResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<FlowScriptDiagnostic>,
    /// Non-blocking acceptance findings surfaced to the human review. They never flip the status
    /// to validation_errors and never block check or commit.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub review_notes: Vec<FlowScriptDiagnostic>,
    /// Deterministic, non-blocking aliases used during reconciliation. The retained source remains
    /// model-authored, so callers should apply these exact rewrites on their next patch.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub corrections: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived_command_count: Option<usize>,
    pub queued_count: usize,
    /// Host-only batch. Model-facing rendering puts it through the existing `<commands>` review
    /// boundary, while direct SDK adapters may drain this field without a text round-trip.
    #[serde(skip)]
    pub commands: Vec<BoardCommand>,
}

impl FlowScriptDraftResponse {
    fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status: "error".to_string(),
            code: Some(code.into()),
            message: message.into(),
            draft_id: None,
            revision: None,
            base_fingerprint: None,
            source: None,
            diagnostics: Vec::new(),
            review_notes: Vec::new(),
            corrections: Vec::new(),
            derived_command_count: None,
            queued_count: 0,
            commands: Vec::new(),
        }
    }

    fn for_draft(
        status: &str,
        message: impl Into<String>,
        draft_id: String,
        draft: &StoredFlowScriptDraft,
    ) -> Self {
        Self {
            status: status.to_string(),
            code: None,
            message: message.into(),
            draft_id: Some(draft_id),
            revision: Some(draft.revision),
            base_fingerprint: Some(draft.base_fingerprint.clone()),
            source: Some(draft.source.clone()),
            diagnostics: draft.evaluation.diagnostics.clone(),
            review_notes: draft.evaluation.review_notes.clone(),
            corrections: draft.evaluation.corrections.clone(),
            derived_command_count: Some(draft.evaluation.commands.len()),
            queued_count: 0,
            commands: Vec::new(),
        }
    }

    fn revision_conflict(
        draft_id: String,
        current_revision: u64,
        expected_revision: u64,
        draft: &StoredFlowScriptDraft,
    ) -> Self {
        let mut response = Self::for_draft(
            "error",
            format!(
                "expected revision {expected_revision}, but current revision is {current_revision}"
            ),
            draft_id,
            draft,
        );
        response.code = Some("FLOWSCRIPT_REVISION_CONFLICT".to_string());
        response
    }

    /// Render the retained source on every successful write/patch/check so a streaming client can
    /// preview the same FlowScript document inline. Queued commits reuse the existing command tag.
    pub fn render_for_model(&self, board: &Board) -> String {
        if self.status == "queued"
            && let Some(source) = self.source.as_deref()
        {
            let result = ReconcileResult {
                commands: self.commands.clone(),
                corrections: self.corrections.clone(),
                diagnostics: Vec::new(),
            };
            let legacy =
                render_edit_flowscript_result(source, &result, board_has_no_nodes(board), true);
            let envelope = serde_json::to_string(self).unwrap_or_else(|_| self.message.clone());
            return format!(
                "{legacy}\n<flowscript_commit_result>{envelope}</flowscript_commit_result>"
            );
        }
        let envelope = serde_json::to_string_pretty(self).unwrap_or_else(|_| self.message.clone());
        match self.source.as_deref() {
            Some(source) => format!(
                "{}\n<flowscript_draft_result>{envelope}</flowscript_draft_result>",
                flowscript_workspace_tag(source, &self.status)
            ),
            None => envelope,
        }
    }
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BeginFlowIrDraftArgs {
    pub draft_id: String,
    /// Explicit opt-in to discard a retained draft with the same id.
    #[serde(default)]
    pub replace_existing: bool,
    pub expected_modules: Vec<String>,
    pub capability_plan: FlowCapabilityPlanRequest,
    /// Defaults to additive so a greenfield IR addition on a non-empty board cannot delete
    /// unrelated work by omission.
    #[serde(default)]
    pub mode: FlowIrDraftMode,
    #[serde(default)]
    pub program: FlowIrProgram,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateFlowIrDraftArgs {
    pub draft_id: String,
    pub expected_revision: u64,
    /// Complete replacement for the retained required-module set. Removing a name is scope
    /// reduction and requires the explicit gate below.
    #[serde(default)]
    pub expected_modules: Option<Vec<String>>,
    /// Complete replacement for the retained capability contract. Removing or weakening a
    /// required capability is scope reduction and requires the explicit gate below.
    #[serde(default)]
    pub capability_plan: Option<FlowCapabilityPlanRequest>,
    /// Complete interface declarations to add or replace by case-insensitive name.
    #[serde(default)]
    pub interfaces: Vec<FlowIrInterface>,
    /// Complete variable declarations to add or replace by case-insensitive name.
    #[serde(default)]
    pub variables: Vec<FlowIrVariable>,
    /// Remove mistakenly authored modules. Removing an expected module is scope reduction.
    #[serde(default)]
    pub remove_modules: Vec<String>,
    #[serde(default)]
    pub remove_interfaces: Vec<String>,
    #[serde(default)]
    pub remove_variables: Vec<String>,
    /// Required when an update intentionally removes expected behavior/header state or replaces an
    /// interface with fewer fields. This must reflect an explicit user request.
    #[serde(default)]
    pub allow_scope_reduction: bool,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpsertFlowIrModuleArgs {
    pub draft_id: String,
    pub expected_revision: u64,
    /// Explicit opt-in for a replacement that contains fewer executable steps than the retained
    /// module. This must only follow an intentional user request to reduce scope.
    #[serde(default)]
    pub allow_scope_reduction: bool,
    pub module: FlowIrModule,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ValidateFlowIrDraftArgs {
    pub draft_id: String,
    /// Include retained interfaces and variables in `retained_ir` for context recovery.
    #[serde(default)]
    pub include_header: bool,
    /// Select retained modules by name for context recovery. Omit to avoid echoing the complete
    /// draft on every validation turn.
    #[serde(default)]
    pub modules: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommitFlowIrDraftArgs {
    pub draft_id: String,
    pub expected_revision: u64,
    /// Legacy compatibility flag. It never authorizes a deletion by itself; replacement commits
    /// must enumerate every derived deletion below.
    #[serde(default)]
    pub allow_deletions: bool,
    /// Exact existing node ids the user authorized this replacement to remove.
    #[serde(default)]
    pub remove_node_ids: Vec<String>,
    /// Exact existing variable ids the user authorized this replacement to remove.
    #[serde(default)]
    pub remove_variable_ids: Vec<String>,
    /// Exact existing function/layer ids the user authorized this replacement to remove.
    #[serde(default)]
    pub remove_layer_ids: Vec<String>,
    /// Exact existing comment ids the user authorized this replacement to remove.
    #[serde(default)]
    pub remove_comment_ids: Vec<String>,
    #[serde(default)]
    pub use_best_candidate: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FlowIrDraftResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<FlowIrDiagnostic>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub module_node_counts: std::collections::BTreeMap<String, usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flowscript: Option<String>,
    /// Selective typed snapshot requested through validate_flow_ir_draft. It is omitted by default
    /// so large drafts do not consume the repair loop's context window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retained_ir: Option<FlowIrProgram>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability_plan: Option<FlowCapabilityPlan>,
    /// Compact whole-draft work remaining while begin/upsert diagnostics stay local to the module
    /// being authored. These become hard `IR_REQUIRED_CAPABILITY_UNUSED` diagnostics at validate.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remaining_capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_modules: Vec<String>,
    /// Commands validation would derive at this revision. Draft operations never queue them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived_command_count: Option<usize>,
    #[serde(skip)]
    pub commands: Vec<BoardCommand>,
}

impl FlowIrDraftResponse {
    fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status: "error".to_string(),
            code: Some(code.into()),
            message: message.into(),
            draft_id: None,
            revision: None,
            base_fingerprint: None,
            diagnostics: Vec::new(),
            module_node_counts: Default::default(),
            flowscript: None,
            retained_ir: None,
            capability_plan: None,
            remaining_capabilities: Vec::new(),
            missing_modules: Vec::new(),
            derived_command_count: None,
            commands: Vec::new(),
        }
    }

    fn revision_conflict(draft_id: String, current: u64, expected: u64) -> Self {
        let mut response = Self::error(
            "IR_REVISION_CONFLICT",
            format!("expected revision {expected}, but current revision is {current}"),
        );
        response.draft_id = Some(draft_id);
        response.revision = Some(current);
        response
    }

    #[allow(clippy::too_many_arguments)]
    fn from_staged_evaluation(
        status: &str,
        draft_id: String,
        revision: u64,
        base_fingerprint: String,
        evaluation: StagedDraftEvaluation,
        capability_plan: Option<FlowCapabilityPlan>,
        missing_modules: Vec<String>,
        pending_capabilities: Vec<String>,
    ) -> Self {
        let diagnostics = evaluation.diagnostics.clone();
        Self::from_staged_evaluation_with_diagnostics(
            status,
            draft_id,
            revision,
            base_fingerprint,
            evaluation,
            diagnostics,
            capability_plan,
            missing_modules,
            pending_capabilities,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_staged_evaluation_with_diagnostics(
        status: &str,
        draft_id: String,
        revision: u64,
        base_fingerprint: String,
        evaluation: StagedDraftEvaluation,
        diagnostics: Vec<FlowIrDiagnostic>,
        capability_plan: Option<FlowCapabilityPlan>,
        missing_modules: Vec<String>,
        pending_capabilities: Vec<String>,
    ) -> Self {
        let structurally_ready = diagnostics.is_empty()
            && missing_modules.is_empty()
            && capability_plan.as_ref().is_none_or(|plan| plan.feasible);
        let message = if structurally_ready && !pending_capabilities.is_empty() {
            "Typed draft is structurally valid at this revision. Continue implementing any known gaps, then run final validation; whole-program capability, request-acceptance, and board reconciliation checks are deliberately deferred until then."
                .to_string()
        } else if structurally_ready {
            "Typed draft is structurally valid at this revision. Whole-program capability, request-acceptance, and board reconciliation checks are deferred to final validation."
                .to_string()
        } else {
            "Repair the structured local diagnostics and missing modules at this same revision; final whole-program validation remains deferred."
                .to_string()
        };
        Self {
            status: status.to_string(),
            code: None,
            message,
            draft_id: Some(draft_id),
            revision: Some(revision),
            base_fingerprint: Some(base_fingerprint),
            diagnostics,
            module_node_counts: evaluation.compile.module_node_counts,
            flowscript: Some(evaluation.compile.flowscript),
            retained_ir: None,
            capability_plan,
            remaining_capabilities: pending_capabilities,
            missing_modules,
            derived_command_count: None,
            commands: Vec::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn from_evaluation(
        status: &str,
        draft_id: String,
        revision: u64,
        base_fingerprint: String,
        evaluation: EvaluatedDraft,
        capability_plan: Option<FlowCapabilityPlan>,
        missing_modules: Vec<String>,
    ) -> Self {
        // Final validation has already merged completion diagnostics into the root diagnostic
        // stream. Derive the compact capability repair list from that merged result.
        let remaining_capabilities =
            remaining_capability_ids(&evaluation.diagnostics, capability_plan.as_ref());
        let structurally_ready = evaluation.diagnostics.is_empty()
            && missing_modules.is_empty()
            && capability_plan.as_ref().is_none_or(|plan| plan.feasible);
        let message = if structurally_ready && !remaining_capabilities.is_empty() {
            "Typed draft is retained and structurally valid. Continue implementing the compact remaining_capabilities before final validation."
                .to_string()
        } else if structurally_ready {
            "Typed draft is structurally valid at this revision; run final validation before commit."
                .to_string()
        } else {
            "Repair the structured root diagnostics and missing requirements at this same revision."
                .to_string()
        };
        Self {
            status: status.to_string(),
            code: None,
            message,
            draft_id: Some(draft_id),
            revision: Some(revision),
            base_fingerprint: Some(base_fingerprint),
            diagnostics: evaluation.diagnostics,
            module_node_counts: evaluation.compile.module_node_counts,
            flowscript: Some(evaluation.compile.flowscript),
            retained_ir: None,
            capability_plan,
            remaining_capabilities,
            missing_modules,
            derived_command_count: evaluation
                .reconcile
                .as_ref()
                .map(|result| result.commands.len()),
            commands: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FlowIrCommitResult {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    /// Revision whose program was selected when `use_best_candidate` chose retained work. The
    /// regular `revision` remains the current draft revision used for idempotency/release.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_revision: Option<u64>,
    /// Board revision the selected command batch was derived from. Hosts carry this into the
    /// Apply/Dismiss lifecycle token and revalidate it immediately before applying.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_fingerprint: Option<String>,
    /// Unique generation of this pending delivery. Required for every host disposition so a stale
    /// duplicated token cannot resolve a later retry of the same draft revision.
    #[serde(skip)]
    pub claim_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flowscript: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<FlowIrDiagnostic>,
    pub queued_count: usize,
    #[serde(skip)]
    pub commands: Vec<BoardCommand>,
}

impl FlowIrCommitResult {
    fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status: "error".to_string(),
            code: Some(code.into()),
            message: message.into(),
            draft_id: None,
            revision: None,
            selected_revision: None,
            base_fingerprint: None,
            claim_id: None,
            flowscript: None,
            diagnostics: Vec::new(),
            queued_count: 0,
            commands: Vec::new(),
        }
    }

    /// Core/Bits path still extracts commands from tags. SDK adapters can instead drain `commands`.
    pub fn render_for_model(&self, board: &Board, allow_deletions: bool) -> String {
        if self.status == "queued"
            && let Some(flowscript) = self.flowscript.as_deref()
        {
            let result = ReconcileResult {
                commands: self.commands.clone(),
                corrections: Vec::new(),
                diagnostics: Vec::new(),
            };
            let legacy = render_edit_flowscript_result(
                flowscript,
                &result,
                board_has_no_nodes(board),
                // The typed commit boundary already compared every destructive command with its
                // exact per-entity authorization. Do not reapply the raw document's global gate.
                true,
            );
            let envelope = serde_json::to_string(self).unwrap_or_else(|_| self.message.clone());
            return format!("{legacy}\n<typed_commit_result>{envelope}</typed_commit_result>");
        }
        let _ = allow_deletions;
        serde_json::to_string_pretty(self).unwrap_or_else(|_| self.message.clone())
    }
}

fn json_schema<T: JsonSchema>() -> serde_json::Value {
    serde_json::to_value(schema_for!(T)).unwrap_or_else(|_| json!({ "type": "object" }))
}

pub struct PlanFlowIrTool {
    pub provider: Arc<dyn CatalogProvider>,
}

impl Tool for PlanFlowIrTool {
    const NAME: &'static str = "plan_flow_ir";
    type Error = FlowIrToolError;
    type Args = FlowCapabilityPlanRequest;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Resolve every required workflow capability and pin contract against the live catalog, and reject over-budget module plans before generation. Call this before beginning a typed draft. Every required capability must select an exact_node_type before feasible can be true. If an exact live node is unknown, first omit exact_node_type for semantic discovery; selection_required=true returns only protocol/operation/algorithm-compatible candidates. Copy one candidate.node_type into exact_node_type and resubmit the complete plan. Never treat a discovery response as feasible or select a lexical decoy."
                .to_string(),
            parameters: json_schema::<FlowCapabilityPlanRequest>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let catalog = self.provider.get_all_metadata().await;
        serde_json::to_string_pretty(&plan_flow_capabilities(&args, &catalog))
            .map_err(|error| FlowIrToolError(error.to_string()))
    }
}

pub struct BeginFlowIrDraftTool {
    pub board: Arc<Board>,
    pub provider: Arc<dyn CatalogProvider>,
    pub store: Arc<FlowIrDraftStore>,
}

/// Request-scoped variant used by production hosts. The binding is captured in the tool instance
/// and never appears in model-visible arguments or results.
pub struct BoundBeginFlowIrDraftTool {
    pub board: Arc<Board>,
    pub provider: Arc<dyn CatalogProvider>,
    pub store: Arc<FlowIrDraftStore>,
    pub acceptance_binding: FlowIrAcceptanceBinding,
}

impl Tool for BoundBeginFlowIrDraftTool {
    const NAME: &'static str = "begin_flow_ir_draft";
    type Error = FlowIrToolError;
    type Args = BeginFlowIrDraftArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Start or explicitly replace an in-memory typed workflow draft against the current board revision. The host binds the original request acceptance contract automatically."
                .to_string(),
            parameters: json_schema::<BeginFlowIrDraftArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let catalog = self.provider.get_all_metadata().await;
        self.store.observe_board(&self.board);
        serde_json::to_string_pretty(&self.store.begin_with_acceptance_binding(
            &self.board,
            &catalog,
            args,
            &self.acceptance_binding,
        ))
        .map_err(|error| FlowIrToolError(error.to_string()))
    }
}

impl Tool for BeginFlowIrDraftTool {
    const NAME: &'static str = "begin_flow_ir_draft";
    type Error = FlowIrToolError;
    type Args = BeginFlowIrDraftArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Start or explicitly replace an in-memory typed workflow draft against the current board revision. Include the complete variable/interface header, required module names, and the capability plan used for feasibility gating."
                .to_string(),
            parameters: json_schema::<BeginFlowIrDraftArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let catalog = self.provider.get_all_metadata().await;
        self.store.observe_board(&self.board);
        serde_json::to_string_pretty(&self.store.begin(&self.board, &catalog, args))
            .map_err(|error| FlowIrToolError(error.to_string()))
    }
}

pub struct UpdateFlowIrDraftTool {
    pub board: Arc<Board>,
    pub provider: Arc<dyn CatalogProvider>,
    pub store: Arc<FlowIrDraftStore>,
}

impl Tool for UpdateFlowIrDraftTool {
    const NAME: &'static str = "update_flow_ir_draft";
    type Error = FlowIrToolError;
    type Args = UpdateFlowIrDraftArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Repair retained interfaces/variables or remove mistakenly authored modules without replaying valid modules. Complete header entries are upserted by name; an intentional request-scope change atomically replaces expected_modules and capability_plan, and all reductions are explicitly scope-gated."
                .to_string(),
            parameters: json_schema::<UpdateFlowIrDraftArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let catalog = self.provider.get_all_metadata().await;
        self.store.observe_board(&self.board);
        serde_json::to_string_pretty(&self.store.update_draft(&self.board, &catalog, args))
            .map_err(|error| FlowIrToolError(error.to_string()))
    }
}

pub struct UpsertFlowIrModuleTool {
    pub board: Arc<Board>,
    pub provider: Arc<dyn CatalogProvider>,
    pub store: Arc<FlowIrDraftStore>,
}

impl Tool for UpsertFlowIrModuleTool {
    const NAME: &'static str = "upsert_flow_ir_module";
    type Error = FlowIrToolError;
    type Args = UpsertFlowIrModuleArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Add or replace one typed function/Event module, compile the whole draft, and retain the previous revision if diagnostics worsen or executable scope shrinks without explicit user authorization."
                .to_string(),
            parameters: json_schema::<UpsertFlowIrModuleArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let catalog = self.provider.get_all_metadata().await;
        self.store.observe_board(&self.board);
        serde_json::to_string_pretty(&self.store.upsert_module(&self.board, &catalog, args))
            .map_err(|error| FlowIrToolError(error.to_string()))
    }
}

pub struct ValidateFlowIrDraftTool {
    pub board: Arc<Board>,
    pub provider: Arc<dyn CatalogProvider>,
    pub store: Arc<FlowIrDraftStore>,
}

impl Tool for ValidateFlowIrDraftTool {
    const NAME: &'static str = "validate_flow_ir_draft";
    type Error = FlowIrToolError;
    type Args = ValidateFlowIrDraftArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Validate the complete typed draft without queueing any board mutations. Returns JSON-pointer diagnostics, exact types/pins, capability feasibility, module coverage, and actual node counts."
                .to_string(),
            parameters: json_schema::<ValidateFlowIrDraftArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let catalog = self.provider.get_all_metadata().await;
        self.store.observe_board(&self.board);
        serde_json::to_string_pretty(&self.store.validate(&self.board, &catalog, args))
            .map_err(|error| FlowIrToolError(error.to_string()))
    }
}

pub struct CommitFlowIrDraftTool {
    pub board: Arc<Board>,
    pub provider: Arc<dyn CatalogProvider>,
    pub store: Arc<FlowIrDraftStore>,
}

impl Tool for CommitFlowIrDraftTool {
    const NAME: &'static str = "commit_flow_ir_draft";
    type Error = FlowIrToolError;
    type Args = CommitFlowIrDraftArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Atomically and idempotently compile and reconcile a validated typed draft. Refuses stale board/revision state, missing required modules, infeasible capabilities, diagnostics, and implicit deletions."
                .to_string(),
            parameters: json_schema::<CommitFlowIrDraftArgs>(),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let catalog = self.provider.get_all_metadata().await;
        let allow_deletions = args.allow_deletions;
        self.store.observe_board(&self.board);
        let result = self.store.commit(&self.board, &catalog, args);
        Ok(result.render_for_model(&self.board, allow_deletions))
    }
}

#[cfg(test)]
mod tests {
    use super::super::ir::{
        FlowCapabilityRequirement, FlowIrArg, FlowIrLiteral, FlowIrStep, FlowIrValue,
    };
    use super::super::types::PinMetadata;
    use super::*;
    use crate::flow::board::{ExecutionMode, ExecutionStage};
    use crate::flow::execution::LogLevel;
    use crate::flow::pin::ValueType;
    use crate::flow::variable::{Variable, VariableType};
    use flow_like_storage::Path;
    use std::{
        sync::mpsc,
        thread,
        time::{Duration, SystemTime},
    };

    #[test]
    fn typed_parse_errors_share_canonical_structured_repair_hints() {
        let error = serde_json::from_value::<FlowCapabilityPlanRequest>(json!({
            "requirements": "not-an-array"
        }))
        .unwrap_err();
        let payload: serde_json::Value = serde_json::from_str(&render_typed_ir_parse_error(
            "IR_CAPABILITY_PLAN_INVALID",
            "typed capability plan",
            &error,
        ))
        .unwrap();
        assert_eq!(payload["status"], "validation_errors");
        assert_eq!(payload["code"], "IR_CAPABILITY_PLAN_INVALID");
        assert_eq!(payload["schema_hint"]["type_object"]["data_type"], "string");
        assert_eq!(
            payload["schema_hint"]["capability_selection"]["selected"]["exact_node_type"],
            "utils_hash_sha256"
        );
    }

    fn pin(name: &str, data_type: &str) -> PinMetadata {
        PinMetadata {
            name: name.to_string(),
            friendly_name: name.to_string(),
            description: String::new(),
            data_type: data_type.to_string(),
            value_type: "Normal".to_string(),
            default_value: None,
            schema: None,
            is_generic: data_type == "Generic",
            valid_values: None,
            enforce_schema: false,
        }
    }

    fn metadata(name: &str, inputs: Vec<PinMetadata>, outputs: Vec<PinMetadata>) -> NodeMetadata {
        NodeMetadata {
            name: name.to_string(),
            friendly_name: name.to_string(),
            description: name.to_string(),
            inputs,
            outputs,
            category: None,
            required_inputs: Vec::new(),
            companion_nodes: Vec::new(),
            capability_tags: Vec::new(),
        }
    }

    #[test]
    fn catalog_repair_omits_ambiguous_companion_declarations() {
        let mut primary = metadata(
            "primary_call",
            vec![pin("input", "String")],
            vec![pin("output", "String")],
        );
        primary.companion_nodes = vec!["duplicate_companion".to_string()];
        let catalog = vec![
            primary,
            metadata(
                "duplicate_companion",
                vec![pin("left", "String")],
                vec![pin("value", "String")],
            ),
            metadata(
                "duplicate_companion",
                vec![pin("right", "Integer")],
                vec![pin("value", "Integer")],
            ),
        ];

        let (declarations, companions, exact) = catalog_repair_declarations(
            &catalog,
            "primaryCall",
            FlowScriptDiagnosticCode::FsUnknownInputPin,
        );
        assert!(exact);
        assert_eq!(declarations.len(), 1);
        assert!(companions.is_empty());
    }

    fn program(message: &str) -> FlowIrProgram {
        FlowIrProgram {
            modules: vec![FlowIrModule::Event {
                name: "eventsSimple".to_string(),
                node_type: "events_simple".to_string(),
                params: Vec::new(),
                steps: vec![FlowIrStep::Node {
                    id: "message".to_string(),
                    node_type: "string_format".to_string(),
                    args: vec![FlowIrArg {
                        pin: "format_string".to_string(),
                        occurrence: 0,
                        value: FlowIrValue::Literal {
                            value: FlowIrLiteral::String(message.to_string()),
                        },
                    }],
                    continue_from: None,
                    exec_arms: Vec::new(),
                    anchor: None,
                }],
                anchor: None,
            }],
            ..Default::default()
        }
    }

    fn catalog() -> Vec<NodeMetadata> {
        vec![
            metadata(
                "events_simple",
                Vec::new(),
                vec![pin("exec_out", "Execution")],
            ),
            metadata(
                "string_format",
                vec![pin("format_string", "String")],
                vec![pin("string", "String")],
            ),
        ]
    }

    #[test]
    fn flowscript_source_validation_injects_exact_and_candidate_catalog_declarations() {
        let mut imap_fetch = metadata(
            "email_imap_inbox_fetch_mail",
            vec![pin("exec_in", "Execution"), pin("email_ref", "Struct")],
            vec![pin("exec_out", "Execution"), pin("email", "Struct")],
        );
        imap_fetch.companion_nodes = vec![
            "email_imap_connect".to_string(),
            "mail_imap_inbox".to_string(),
            "mail_imap_list".to_string(),
            "email_get_headers".to_string(),
            "email_get_content".to_string(),
            "mail_address_fields".to_string(),
            "email_imap_mark_seen".to_string(),
            "email_imap_move_message".to_string(),
        ];
        let repair_catalog = vec![
            metadata(
                "events_simple",
                Vec::new(),
                vec![pin("exec_out", "Execution")],
            ),
            metadata(
                "string_replace",
                vec![
                    pin("string", "String"),
                    pin("pattern", "String"),
                    pin("replacement", "String"),
                    pin("is_regex", "Boolean"),
                ],
                vec![pin("string", "String")],
            ),
            metadata(
                "email_smtp_send",
                vec![
                    pin("exec_in", "Execution"),
                    pin("connection", "Struct"),
                    pin("from", "String"),
                    pin("to", "String"),
                    pin("body_text", "String"),
                ],
                vec![pin("exec_out", "Execution"), pin("message_id", "String")],
            ),
            imap_fetch,
            metadata(
                "email_imap_connect",
                vec![
                    pin("exec_in", "Execution"),
                    pin("username", "String"),
                    pin("password", "String"),
                ],
                vec![pin("exec_out", "Execution"), pin("connection", "Struct")],
            ),
            metadata(
                "mail_imap_inbox",
                vec![pin("exec_in", "Execution"), pin("connection", "Struct")],
                vec![pin("exec_out", "Execution"), pin("inbox_struct", "Struct")],
            ),
            metadata(
                "mail_imap_list",
                vec![pin("exec_in", "Execution"), pin("inbox", "Struct")],
                vec![pin("exec_out", "Execution"), pin("emails", "Struct")],
            ),
            metadata(
                "email_get_headers",
                vec![pin("email", "Struct")],
                vec![pin("from", "Struct")],
            ),
            metadata(
                "email_get_content",
                vec![pin("email", "Struct")],
                vec![pin("plain", "String"), pin("html", "String")],
            ),
            metadata(
                "mail_address_fields",
                vec![pin("address", "Struct")],
                vec![pin("email", "String")],
            ),
            metadata(
                "email_imap_mark_seen",
                vec![pin("exec_in", "Execution"), pin("email", "Struct")],
                vec![pin("exec_out", "Execution"), pin("email_ref", "Struct")],
            ),
            metadata(
                "email_imap_move_message",
                vec![pin("exec_in", "Execution"), pin("email", "Struct")],
                vec![
                    pin("exec_out", "Execution"),
                    pin("new_message_ref", "Struct"),
                ],
            ),
        ];
        let source = r#"eventsSimple() {
    const replaced = stringReplace({ string: "x", pattern: "x", replacement: "y", regexp: true })
    emailImapInboxFetchMail({ email: null, unseenOnly: true, markSeen: true })
    emailSmtpSendMail({ to: "customer@example.com", body: replaced.string })
}
"#;
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let written = store.write_flowscript(
            &board,
            &repair_catalog,
            WriteFlowScriptArgs {
                draft_id: "catalog-repair-context".to_string(),
                replace_existing: false,
                mode: FlowIrDraftMode::Additive,
                source: source.to_string(),
                allow_scope_reduction: false,
            },
        );
        assert_eq!(written.status, "validation_errors", "{written:#?}");
        {
            let drafts = store.source_drafts.lock().expect("source draft store");
            let retained = drafts
                .get("catalog-repair-context")
                .expect("retained source draft");
            let retained_size = stored_flowscript_draft_size(retained);
            let mut without_repair_context = retained.clone();
            without_repair_context.evaluation.diagnostics.clear();
            without_repair_context.evaluation.corrections.clear();
            assert!(
                retained_size > stored_flowscript_draft_size(&without_repair_context),
                "retained byte accounting must include serialized repair diagnostics"
            );
        }

        let assert_repair_context = |response: &FlowScriptDraftResponse| {
            let bad_pin = response
                .diagnostics
                .iter()
                .find(|diagnostic| {
                    diagnostic.code == FlowScriptDiagnosticCode::FsUnknownInputPin
                        && diagnostic.declaration.as_deref() == Some("stringReplace")
                })
                .expect("known call should report its unknown input pin");
            let bad_pin_fix = bad_pin
                .fix
                .as_ref()
                .expect("bad pin should have a repair fix");
            assert_eq!(bad_pin_fix.catalog_declarations.len(), 1);
            assert!(
                bad_pin_fix.catalog_declarations[0].contains("declare function stringReplace(")
            );
            assert!(bad_pin_fix.catalog_declarations[0].contains("isRegex: bool"));
            assert!(bad_pin_fix.declaration_search.is_none());

            let structural_imap_repair = response
                .diagnostics
                .iter()
                .find(|diagnostic| {
                    diagnostic.code == FlowScriptDiagnosticCode::FsUnknownInputPin
                        && diagnostic.declaration.as_deref() == Some("emailImapInboxFetchMail")
                })
                .and_then(|diagnostic| diagnostic.fix.as_ref())
                .expect("IMAP fetch pin failure should carry structural repair context");
            assert_eq!(structural_imap_repair.catalog_declarations.len(), 1);
            assert!(
                structural_imap_repair.catalog_declarations[0]
                    .contains("emailImapInboxFetchMail({ emailRef: Struct })")
            );
            assert_eq!(structural_imap_repair.companion_declarations.len(), 8);
            for companion in [
                "emailImapConnect",
                "mailImapInbox",
                "mailImapList",
                "emailGetHeaders",
                "emailGetContent",
                "mailAddressFields",
                "emailImapMarkSeen",
            ] {
                assert!(
                    structural_imap_repair
                        .companion_declarations
                        .iter()
                        .any(|declaration| declaration.contains(companion)),
                    "missing structural companion {companion}: {:?}",
                    structural_imap_repair.companion_declarations
                );
            }

            let unknown_call = response
                .diagnostics
                .iter()
                .find(|diagnostic| {
                    diagnostic.code == FlowScriptDiagnosticCode::FsCatalogDeclarationNotFound
                        && diagnostic.declaration.as_deref() == Some("emailSmtpSendMail")
                })
                .expect("unknown call should remain a blocking diagnostic");
            let unknown_call_fix = unknown_call
                .fix
                .as_ref()
                .expect("unknown call should have bounded catalog candidates");
            assert!(!unknown_call_fix.catalog_declarations.is_empty());
            assert!(
                unknown_call_fix.catalog_declarations[0]
                    .contains("declare function emailSmtpSend(")
            );
            assert!(unknown_call_fix.catalog_declarations.len() <= 3);
        };

        assert_repair_context(&written);
        let model_output = written.render_for_model(&board);
        assert!(model_output.contains("\"catalog_declarations\""));
        assert!(model_output.contains("\"companion_declarations\""));
        assert!(model_output.contains("declare function emailSmtpSend("));
        assert!(model_output.contains("declare function mailImapList("));
        let patched = store.patch_flowscript(
            &board,
            &repair_catalog,
            PatchFlowScriptArgs {
                draft_id: "catalog-repair-context".to_string(),
                expected_revision: 0,
                old_text: "customer@example.com".to_string(),
                new_text: "support@example.com".to_string(),
                allow_scope_reduction: false,
            },
        );
        assert_eq!(patched.status, "validation_errors", "{patched:#?}");
        assert_eq!(patched.revision, Some(1));
        assert_repair_context(&patched);
        let checked = store.check_flowscript(
            &board,
            &repair_catalog,
            CheckFlowScriptArgs {
                draft_id: "catalog-repair-context".to_string(),
                expected_revision: 1,
            },
        );
        assert_eq!(checked.status, "validation_errors", "{checked:#?}");
        assert_repair_context(&checked);
    }

    #[test]
    fn flowscript_source_responses_retain_and_serialize_canonical_corrections() {
        let catalog = vec![
            metadata(
                "events_simple",
                Vec::new(),
                vec![pin("exec_out", "Execution")],
            ),
            metadata(
                "string_replace",
                vec![
                    pin("string", "String"),
                    pin("pattern", "String"),
                    pin("replacement", "String"),
                    pin("is_regex", "Boolean"),
                ],
                vec![pin("string", "String")],
            ),
        ];
        let source = r#"eventsSimple() {
    const replaced = stringReplace({ string: "abc", pattern: "a", replacement: "z", regex: true })
}
"#;
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let written = store.write_flowscript(
            &board,
            &catalog,
            WriteFlowScriptArgs {
                draft_id: "canonical-corrections".to_string(),
                replace_existing: false,
                mode: FlowIrDraftMode::Additive,
                source: source.to_string(),
                allow_scope_reduction: false,
            },
        );
        assert!(written.diagnostics.is_empty(), "{written:#?}");
        assert_eq!(written.corrections.len(), 1, "{written:#?}");
        assert!(written.corrections[0].contains("`regex` to `isRegex`"));
        assert!(written.render_for_model(&board).contains("\"corrections\""));

        let patched = store.patch_flowscript(
            &board,
            &catalog,
            PatchFlowScriptArgs {
                draft_id: "canonical-corrections".to_string(),
                expected_revision: 0,
                old_text: "abc".to_string(),
                new_text: "abcd".to_string(),
                allow_scope_reduction: false,
            },
        );
        assert_eq!(patched.revision, Some(1));
        assert_eq!(patched.corrections, written.corrections);

        let checked = store.check_flowscript(
            &board,
            &catalog,
            CheckFlowScriptArgs {
                draft_id: "canonical-corrections".to_string(),
                expected_revision: 1,
            },
        );
        assert_eq!(checked.corrections, written.corrections);
        assert!(checked.render_for_model(&board).contains("isRegex"));
    }

    fn capability_plan() -> FlowCapabilityPlanRequest {
        FlowCapabilityPlanRequest {
            requirements: vec![FlowCapabilityRequirement {
                id: "format_message".to_string(),
                intent: "format a message".to_string(),
                required: true,
                exact_node_type: Some("string_format".to_string()),
                inputs: Vec::new(),
                outputs: Vec::new(),
            }],
            modules: vec![
                super::super::ir::FlowModuleEstimate {
                    name: "eventsSimple".to_string(),
                    kind: FlowModuleKind::Event,
                    estimated_nodes: 1,
                },
                super::super::ir::FlowModuleEstimate {
                    name: "classify".to_string(),
                    kind: FlowModuleKind::Function,
                    estimated_nodes: 1,
                },
            ],
        }
    }

    fn acceptance_catalog() -> Vec<NodeMetadata> {
        let mut catalog = catalog();
        catalog.push(metadata(
            "slack_send",
            vec![pin("exec_in", "Execution"), pin("message", "String")],
            vec![pin("exec_out", "Execution")],
        ));
        catalog
    }

    fn acceptance_capability_plan(include_slack: bool) -> FlowCapabilityPlanRequest {
        let mut requirements = vec![FlowCapabilityRequirement {
            id: "format_customer_message".to_string(),
            intent: "format the customer message".to_string(),
            required: true,
            exact_node_type: Some("string_format".to_string()),
            inputs: Vec::new(),
            outputs: Vec::new(),
        }];
        if include_slack {
            requirements.push(FlowCapabilityRequirement {
                id: "send_slack_notification".to_string(),
                intent: "send a Slack notification".to_string(),
                required: true,
                exact_node_type: Some("slack_send".to_string()),
                inputs: Vec::new(),
                outputs: Vec::new(),
            });
        }
        FlowCapabilityPlanRequest {
            requirements,
            modules: vec![super::super::ir::FlowModuleEstimate {
                name: "eventsSimple".to_string(),
                kind: FlowModuleKind::Event,
                estimated_nodes: 2,
            }],
        }
    }

    fn acceptance_program() -> FlowIrProgram {
        let mut program = program("hello");
        let FlowIrModule::Event { steps, .. } = &mut program.modules[0] else {
            unreachable!("test program is an event")
        };
        steps.push(FlowIrStep::Node {
            id: "notify_slack".to_string(),
            node_type: "slack_send".to_string(),
            args: vec![FlowIrArg {
                pin: "message".to_string(),
                occurrence: 0,
                value: FlowIrValue::Literal {
                    value: FlowIrLiteral::String("customer message".to_string()),
                },
            }],
            continue_from: None,
            exec_arms: Vec::new(),
            anchor: None,
        });
        program
    }

    fn empty_board() -> Board {
        Board {
            id: "board".to_string(),
            name: "Board".to_string(),
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

    fn flowscript_catalog() -> Vec<NodeMetadata> {
        vec![
            metadata(
                "events_simple",
                Vec::new(),
                vec![pin("exec_out", "Execution")],
            ),
            metadata(
                "log_info",
                vec![pin("exec_in", "Execution"), pin("message", "String")],
                vec![pin("exec_out", "Execution")],
            ),
        ]
    }

    fn valid_flowscript(message: &str) -> String {
        format!("eventsSimple() {{\n    logInfo({{ message: {message:?} }})\n}}\n")
    }

    fn acceptance_flowscript_catalog() -> Vec<NodeMetadata> {
        let mut catalog = acceptance_catalog();
        catalog.push(metadata(
            "email_send",
            vec![pin("exec_in", "Execution"), pin("message", "String")],
            vec![pin("exec_out", "Execution")],
        ));
        catalog
    }

    fn complete_acceptance_flowscript(include_email: bool) -> String {
        format!(
            "eventsSimple() {{\n    const formatted = stringFormat({{ formatString: \"customer message\" }})\n    slackSend({{ message: formatted.string }})\n{} }}\n",
            if include_email {
                "    emailSend({ message: formatted.string })\n"
            } else {
                ""
            }
        )
    }

    fn approval_flowscript_catalog() -> Vec<NodeMetadata> {
        vec![
            metadata(
                "events_simple",
                Vec::new(),
                vec![pin("exec_out", "Execution")],
            ),
            metadata(
                "imap_email_fetch",
                vec![pin("exec_in", "Execution")],
                vec![
                    pin("exec_out", "Execution"),
                    pin("ticket_id", "String"),
                    pin("incoming_sender", "String"),
                    pin("reviewer_approval_decision", "String"),
                    pin("requester_email", "String"),
                    pin("reviewer_change_feedback", "String"),
                ],
            ),
            metadata(
                "ai_model_generate",
                vec![
                    pin("exec_in", "Execution"),
                    pin("reviewer_feedback", "String"),
                ],
                vec![
                    pin("exec_out", "Execution"),
                    pin("text", "String"),
                    pin("draft_version", "String"),
                ],
            ),
            metadata(
                "smtp_email_send",
                vec![
                    pin("exec_in", "Execution"),
                    pin("to", "String"),
                    pin("body", "String"),
                    pin("ticket_id", "String"),
                    pin("draft_version", "String"),
                ],
                vec![pin("exec_out", "Execution")],
            ),
            metadata(
                "reviewer_decision_validate",
                vec![
                    pin("exec_in", "Execution"),
                    pin("incoming_sender", "String"),
                    pin("decision", "String"),
                    pin("ticket_id", "String"),
                    pin("draft_version", "String"),
                    pin("expected_reviewer", "String"),
                ],
                vec![
                    pin("exec_out", "Execution"),
                    pin("approved_reviewer_decision", "Boolean"),
                ],
            ),
            metadata(
                "control_branch",
                vec![pin("exec_in", "Execution"), pin("condition", "Boolean")],
                vec![pin("true", "Execution"), pin("false", "Execution")],
            ),
        ]
    }

    fn complete_approval_flowscript() -> String {
        r#"eventsSimple() {
    const fetched = imapEmailFetch()
    const draft = aiModelGenerate()
    smtpEmailSend({ to: "example@example.com", body: draft.text, ticketId: fetched.ticketId, draftVersion: draft.draftVersion })
    const validation = reviewerDecisionValidate({ incomingSender: fetched.incomingSender, decision: fetched.reviewerApprovalDecision, ticketId: fetched.ticketId, draftVersion: draft.draftVersion, expectedReviewer: "example@example.com" })
    if (validation.approvedReviewerDecision) {
        smtpEmailSend({ to: fetched.requesterEmail, body: draft.text, ticketId: fetched.ticketId, draftVersion: draft.draftVersion })
    } else {
        const revised = aiModelGenerate({ reviewerFeedback: fetched.reviewerChangeFeedback })
        smtpEmailSend({ to: "example@example.com", body: revised.text, ticketId: fetched.ticketId, draftVersion: revised.draftVersion })
    }
}
"#
        .to_string()
    }

    fn check_bound_flowscript(
        request: &str,
        source: String,
        catalog: &[NodeMetadata],
    ) -> (
        FlowIrDraftStore,
        Board,
        FlowIrAcceptanceBinding,
        FlowScriptDraftResponse,
    ) {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let binding = store.bind_request_acceptance_contract(&board.id, request);
        let written = store.write_flowscript_with_acceptance_binding(
            &board,
            catalog,
            WriteFlowScriptArgs {
                draft_id: "source-acceptance".to_string(),
                replace_existing: false,
                mode: FlowIrDraftMode::Additive,
                source,
                allow_scope_reduction: false,
            },
            &binding,
        );
        assert_eq!(written.revision, Some(0), "{written:#?}");
        let checked = store.check_flowscript_with_acceptance_binding(
            &board,
            catalog,
            CheckFlowScriptArgs {
                draft_id: "source-acceptance".to_string(),
                expected_revision: 0,
            },
            &binding,
        );
        (store, board, binding, checked)
    }

    #[test]
    fn flowscript_check_enforces_positive_multi_capability_request_scope() {
        let request = "Format the customer message, then send a Slack notification.";
        let (_, _, _, checked) = check_bound_flowscript(
            request,
            complete_acceptance_flowscript(false),
            &acceptance_flowscript_catalog(),
        );
        assert_eq!(checked.status, "valid", "{checked:#?}");
        assert!(checked.diagnostics.is_empty());
    }

    #[test]
    fn flowscript_acceptance_counts_only_helpers_reachable_from_an_event() {
        let request = derive_request_acceptance_contract(
            "Format the customer message, then send a Slack notification.",
        );
        let unreachable = flow_like_ast::parse(
            r#"function notifySlack(message: string) {
    slackSend({ message: message })
}
eventsSimple() {
    const formatted = stringFormat({ formatString: "customer message" })
}
"#,
        )
        .expect("unreachable helper source parses");
        let catalog = acceptance_flowscript_catalog();
        assert!(
            flowscript_acceptance_diagnostics(&request, &unreachable, &catalog)
                .iter()
                .any(|diagnostic| diagnostic.code
                    == FlowScriptDiagnosticCode::FsRequestAcceptanceIncomplete)
        );

        let reachable = flow_like_ast::parse(
            r#"function notifySlack(message: string) {
    slackSend({ message: message })
}
eventsSimple() {
    const formatted = stringFormat({ formatString: "customer message" })
    notifySlack({ message: formatted.string })
}
"#,
        )
        .expect("called helper source parses");
        assert!(flowscript_acceptance_diagnostics(&request, &reachable, &catalog).is_empty());
    }

    #[test]
    fn incomplete_scope_findings_become_review_notes_and_never_block_commit() {
        let request = "Format the customer message, then send a Slack notification.";
        let source =
            "eventsSimple() {\n    slackSend({ message: \"customer message\" })\n}\n".to_string();
        let (store, board, binding, checked) =
            check_bound_flowscript(request, source.clone(), &acceptance_flowscript_catalog());
        assert_eq!(checked.status, "valid", "{checked:#?}");
        assert_eq!(checked.source.as_deref(), Some(source.as_str()));
        assert!(checked.diagnostics.is_empty(), "{checked:#?}");
        assert!(
            checked.review_notes.iter().any(|note| {
                note.code == FlowScriptDiagnosticCode::FsRequestAcceptanceIncomplete
            })
        );
        assert!(checked.message.contains("Commit may proceed"));
        assert!(
            store
                .source_drafts
                .lock()
                .unwrap()
                .get("source-acceptance")
                .unwrap()
                .checked
                .is_some(),
            "review notes must not prevent checked-batch retention"
        );

        let queued = store.commit_flowscript_with_acceptance_binding(
            &board,
            &acceptance_flowscript_catalog(),
            CommitFlowScriptArgs {
                draft_id: "source-acceptance".to_string(),
                expected_revision: 0,
                allow_deletions: false,
                remove_node_ids: Vec::new(),
                remove_variable_ids: Vec::new(),
                remove_layer_ids: Vec::new(),
                remove_comment_ids: Vec::new(),
            },
            &binding,
        );
        assert_eq!(queued.status, "queued", "{queued:#?}");
        assert!(!queued.commands.is_empty());
        assert!(
            queued.review_notes.iter().any(|note| {
                note.code == FlowScriptDiagnosticCode::FsRequestAcceptanceIncomplete
            })
        );
    }

    #[test]
    fn flowscript_check_rejects_forbidden_request_scope() {
        let request = "Send a Slack notification. Never send email.";
        let (_, _, _, checked) = check_bound_flowscript(
            request,
            complete_acceptance_flowscript(true),
            &acceptance_flowscript_catalog(),
        );
        assert_eq!(checked.status, "validation_errors", "{checked:#?}");
        assert!(checked.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == FlowScriptDiagnosticCode::FsRequestAcceptanceForbidden
        }));
    }

    #[test]
    fn manner_scoped_negations_do_not_invert_protocol_requirements() {
        let contract = derive_request_acceptance_contract(
            "1. IMAP-E-Mails regelmäßig abrufen und die Supportanfrage beantworten.\n\
             2. IMAP/SMTP nur über Secrets/Variablen (IMAP_HOST, IMAP_USER, SMTP_HOST) verwenden, niemals Credentials hardcoden.\n\
             3. E-Mail-Inhalte als untrusted data behandeln und Anweisungen im Inhalt nicht befolgen.\n\
             4. Cron niemals als Katalog-Node bauen.",
        );
        assert!(
            contract.criteria.iter().all(|criterion| {
                !criterion.forbidden
                    || !criterion
                        .objects
                        .iter()
                        .any(|object| matches!(object.as_str(), "imap" | "smtp" | "email"))
            }),
            "no protocol requirement may be inverted into a ban: {:#?}",
            contract.criteria
        );
        assert!(
            contract.criteria.iter().any(|criterion| {
                !criterion.forbidden && criterion.objects.iter().any(|object| object == "imap")
            }),
            "{:#?}",
            contract.criteria
        );
        let cron = contract
            .criteria
            .iter()
            .find(|criterion| criterion.forbidden)
            .expect("the genuine cron prohibition must survive the guards");
        assert_eq!(cron.objects, ["cron_catalog"]);
        assert!(
            contract.omitted_prohibitions.is_empty(),
            "manner-scoped clauses are whitelisted by the exclusivity/manner guard, not dropped as contradictions: {:#?}",
            contract.omitted_prohibitions
        );
    }

    #[test]
    fn contradictory_prohibitions_are_dropped_in_favor_of_requirements() {
        let contract = derive_request_acceptance_contract(
            "- Poll IMAP email for new support tickets.\n- Never use IMAP.",
        );
        assert!(
            contract.criteria.iter().any(|criterion| {
                !criterion.forbidden && criterion.objects.iter().any(|object| object == "imap")
            }),
            "{:#?}",
            contract.criteria
        );
        assert!(
            contract
                .criteria
                .iter()
                .all(|criterion| !criterion.forbidden),
            "a contract must never require and ban the same subject: {:#?}",
            contract.criteria
        );
        assert_eq!(
            contract.omitted_prohibitions.len(),
            1,
            "a dropped contradiction must be traced for the human review: {:#?}",
            contract.omitted_prohibitions
        );
    }

    #[test]
    fn action_scoped_bans_survive_requirements_that_merely_read_the_subject() {
        let contract = derive_request_acceptance_contract(
            "Lies E-Mails per IMAP. Sende niemals E-Mails per SMTP.",
        );
        assert!(
            contract.criteria.iter().any(|criterion| {
                !criterion.forbidden && criterion.objects.iter().any(|object| object == "imap")
            }),
            "IMAP polling must stay required: {:#?}",
            contract.criteria
        );
        let ban = contract
            .criteria
            .iter()
            .find(|criterion| criterion.forbidden)
            .expect("the SMTP send ban shares only the read subject and must survive");
        assert!(
            ban.actions.iter().any(|action| action == "send"),
            "{ban:#?}"
        );
        assert!(
            ban.objects.iter().any(|object| object == "smtp"),
            "{ban:#?}"
        );
    }

    #[test]
    fn transfer_scoped_credential_bans_are_never_inverted_into_requirements() {
        for request in [
            "Sende niemals Credentials per E-Mail.",
            "Never send credentials by email.",
        ] {
            let contract = derive_request_acceptance_contract(request);
            assert!(
                !contract.criteria.iter().any(|criterion| {
                    !criterion.forbidden && criterion.objects.iter().any(|object| object == "email")
                }),
                "a credential-exfiltration ban must never become a positive email requirement for {request:?}: {:#?}",
                contract.criteria
            );
            let ban = contract
                .criteria
                .iter()
                .find(|criterion| criterion.forbidden)
                .unwrap_or_else(|| panic!("the transfer-scoped ban must survive for {request:?}"));
            assert!(
                ban.actions.iter().any(|action| action == "send"),
                "{ban:#?}"
            );
            assert_eq!(ban.objects, ["email"], "{ban:#?}");
        }
    }

    #[test]
    fn unenforceable_prohibitions_are_traced_for_human_review() {
        let contract = derive_request_acceptance_contract(
            "- Send a Slack notification for every ticket.\n- Never email the customer directly.",
        );
        assert!(
            contract
                .criteria
                .iter()
                .all(|criterion| !criterion.forbidden),
            "recipient-scoped bans need dataflow proof and must not become protocol bans: {:#?}",
            contract.criteria
        );
        assert_eq!(contract.omitted_prohibitions.len(), 1);
        assert!(
            contract.omitted_prohibitions[0].contains("Never email the customer directly"),
            "{:#?}",
            contract.omitted_prohibitions
        );
    }

    #[test]
    fn check_surfaces_prohibitions_the_machine_could_not_enforce() {
        let request = "Send a Slack notification. Never send email to the customer directly.";
        let (_, _, _, checked) = check_bound_flowscript(
            request,
            complete_acceptance_flowscript(false),
            &acceptance_flowscript_catalog(),
        );
        assert_eq!(checked.status, "valid", "{checked:#?}");
        assert!(
            checked.message.contains("could not be machine-enforced"),
            "{}",
            checked.message
        );
        assert!(
            checked
                .message
                .contains("Never send email to the customer directly"),
            "{}",
            checked.message
        );
    }

    #[test]
    fn near_identical_send_clauses_collapse_to_one_criterion() {
        let contract = derive_request_acceptance_contract(
            "- Send a summary email with delivery stats.\n\
             - Send a summary email with delivery statistics to the team.",
        );
        let send_email_criteria = contract
            .criteria
            .iter()
            .filter(|criterion| {
                !criterion.forbidden
                    && criterion.actions == ["send"]
                    && criterion.objects == ["email"]
            })
            .count();
        assert_eq!(send_email_criteria, 1, "{:#?}", contract.criteria);
    }

    #[test]
    fn bounded_summary_cuts_on_word_boundaries_with_an_ellipsis_marker() {
        let short = bounded_summary("Send a Slack notification.");
        assert_eq!(short, "Send a Slack notification.");

        let words = ["Sende", "eine", "Statusmail", "mit", "Fehlerstatistik"];
        let long = words.join(" ").repeat(20);
        let long = long.replace("FehlerstatistikSende", "Fehlerstatistik Sende");
        let summary = bounded_summary(&long);
        assert!(summary.chars().count() <= MAX_FLOW_IR_ACCEPTANCE_SUMMARY_CHARS);
        assert!(summary.ends_with(" …"), "{summary:?}");
        let last_word = summary
            .trim_end_matches(" …")
            .split_whitespace()
            .next_back()
            .expect("summary keeps at least one word");
        assert!(
            words.contains(&last_word),
            "summary must end on a whole word: {summary:?}"
        );
    }

    #[test]
    fn check_reuses_the_stored_evaluation_until_the_catalog_moves() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let catalog = flowscript_catalog();
        assert_eq!(store.global_evaluation_count(), 0);
        let written = store.write_flowscript(
            &board,
            &catalog,
            WriteFlowScriptArgs {
                draft_id: "single-eval".to_string(),
                replace_existing: false,
                mode: FlowIrDraftMode::Additive,
                source: valid_flowscript("hello"),
                allow_scope_reduction: false,
            },
        );
        assert_eq!(written.revision, Some(0), "{written:#?}");
        assert_eq!(store.global_evaluation_count(), 1);

        let checked = store.check_flowscript(
            &board,
            &catalog,
            CheckFlowScriptArgs {
                draft_id: "single-eval".to_string(),
                expected_revision: 0,
            },
        );
        assert_eq!(checked.status, "valid", "{checked:#?}");
        assert_eq!(
            store.global_evaluation_count(),
            1,
            "check must reuse the evaluation stored by write when board and catalog are unchanged"
        );

        let mut changed_catalog = catalog;
        changed_catalog[1].outputs.push(pin("status", "String"));
        let rechecked = store.check_flowscript(
            &board,
            &changed_catalog,
            CheckFlowScriptArgs {
                draft_id: "single-eval".to_string(),
                expected_revision: 0,
            },
        );
        assert_eq!(rechecked.status, "valid", "{rechecked:#?}");
        assert_eq!(
            store.global_evaluation_count(),
            2,
            "a live catalog change must trigger exactly one re-evaluation"
        );
    }

    #[test]
    fn failed_patch_keeps_the_last_checked_revision_committable() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let catalog = flowscript_catalog();
        let written = store.write_flowscript(
            &board,
            &catalog,
            WriteFlowScriptArgs {
                draft_id: "source-salvage".to_string(),
                replace_existing: false,
                mode: FlowIrDraftMode::Additive,
                source: valid_flowscript("hello"),
                allow_scope_reduction: false,
            },
        );
        assert_eq!(written.revision, Some(0), "{written:#?}");
        let checked = store.check_flowscript(
            &board,
            &catalog,
            CheckFlowScriptArgs {
                draft_id: "source-salvage".to_string(),
                expected_revision: 0,
            },
        );
        assert_eq!(checked.status, "valid", "{checked:#?}");
        let checked_commands = store
            .source_drafts
            .lock()
            .unwrap()
            .get("source-salvage")
            .unwrap()
            .checked
            .as_ref()
            .unwrap()
            .commands
            .clone();

        let broken = store.patch_flowscript(
            &board,
            &catalog,
            PatchFlowScriptArgs {
                draft_id: "source-salvage".to_string(),
                expected_revision: 0,
                old_text: "logInfo".to_string(),
                new_text: "logInfoo".to_string(),
                allow_scope_reduction: false,
            },
        );
        assert_eq!(broken.status, "validation_errors", "{broken:#?}");
        assert_eq!(broken.revision, Some(1));

        let stale = store.commit_flowscript(
            &board,
            &catalog,
            CommitFlowScriptArgs {
                draft_id: "source-salvage".to_string(),
                expected_revision: 3,
                allow_deletions: false,
                remove_node_ids: Vec::new(),
                remove_variable_ids: Vec::new(),
                remove_layer_ids: Vec::new(),
                remove_comment_ids: Vec::new(),
            },
        );
        assert_eq!(stale.code.as_deref(), Some("FLOWSCRIPT_REVISION_CONFLICT"));

        let queued = store.commit_flowscript(
            &board,
            &catalog,
            CommitFlowScriptArgs {
                draft_id: "source-salvage".to_string(),
                expected_revision: 0,
                allow_deletions: false,
                remove_node_ids: Vec::new(),
                remove_variable_ids: Vec::new(),
                remove_layer_ids: Vec::new(),
                remove_comment_ids: Vec::new(),
            },
        );
        assert_eq!(queued.status, "queued", "{queued:#?}");
        assert_eq!(queued.revision, Some(0));
        assert!(queued.message.contains("restored"), "{queued:#?}");
        assert!(
            queued
                .source
                .as_deref()
                .is_some_and(|source| source.contains("logInfo(")),
            "the exact checked source is restored: {queued:#?}"
        );
        assert_eq!(
            serde_json::to_value(&queued.commands).unwrap(),
            serde_json::to_value(&checked_commands).unwrap()
        );

        let token = store
            .latest_pending_commit_token(&board.id)
            .expect("salvage commit produces a standard pending claim");
        assert_eq!(token.revision, 0);
        let retained = store
            .pending_commands_if_current(
                &board,
                &token.draft_id,
                token.revision,
                &token.base_fingerprint,
                &token.claim_id,
            )
            .expect("salvaged batch resolves through the shared claim API");
        assert_eq!(
            serde_json::to_value(retained).unwrap(),
            serde_json::to_value(checked_commands).unwrap()
        );

        let blocked = store.patch_flowscript(
            &board,
            &catalog,
            PatchFlowScriptArgs {
                draft_id: "source-salvage".to_string(),
                expected_revision: 0,
                old_text: "hello".to_string(),
                new_text: "goodbye".to_string(),
                allow_scope_reduction: false,
            },
        );
        assert_eq!(
            blocked.code.as_deref(),
            Some("FLOWSCRIPT_DRAFT_COMMIT_PENDING"),
            "{blocked:#?}"
        );
    }

    #[test]
    fn flowscript_check_proves_correlated_reviewer_change_and_reask_loop() {
        let request = "Bau mir eine App:\n\
            1. IMAP-E-Mails abrufen -> Das Modell beantwortet die Supportanfrage\n\
            2. Eine Freigabemail an example@example.com senden\n\
            3. Bei Freigabe eine Kundenmail senden\n\
            4. Sonst den Entwurf anpassen und erneut eine Freigabe anfragen";
        let (_, _, _, checked) = check_bound_flowscript(
            request,
            complete_approval_flowscript(),
            &approval_flowscript_catalog(),
        );
        assert_eq!(checked.status, "valid", "{checked:#?}");
        assert!(checked.diagnostics.is_empty());
    }

    const LAYERED_APPROVAL_REQUEST: &str = "Bau mir eine App:\n\
        1. IMAP-E-Mails abrufen -> Das Modell beantwortet die Supportanfrage\n\
        2. Eine Freigabemail an example@example.com senden\n\
        3. Bei Freigabe eine Kundenmail senden\n\
        4. Sonst den Entwurf anpassen und erneut eine Freigabe anfragen";

    /// Event -> processMailbox -> loop -> processOneMail -> sendFinal: every approval obligation
    /// (reviewer send, sender-literal comparison, revision correlation, approved-branch customer
    /// send, rejection-branch regenerate + re-send) lives inside helper functions.
    fn layered_approval_flowscript(include_sender_check: bool) -> String {
        let sender_check = if include_sender_check {
            "mail.incomingSender == APPROVER_EMAIL && "
        } else {
            ""
        };
        format!(
            r#"const APPROVER_EMAIL: string = "example@example.com"

function sendFinal(mail: object, draft: object) {{
    smtpEmailSend({{ to: mail.requesterEmail, body: draft.text, ticketId: mail.ticketId, draftVersion: draft.draftVersion }})
}}
function processOneMail(mail: object) {{
    const draft = aiModelGenerate()
    smtpEmailSend({{ to: APPROVER_EMAIL, body: draft.text, ticketId: mail.ticketId, draftVersion: draft.draftVersion }})
    if ({sender_check}mail.reviewerApprovalDecision == "approved" && mail.ticketId == draft.ticketId && mail.draftVersion == draft.draftVersion) {{
        sendFinal({{ mail: mail, draft: draft }})
    }} else {{
        const revised = aiModelGenerate({{ reviewerFeedback: mail.reviewerChangeFeedback }})
        smtpEmailSend({{ to: APPROVER_EMAIL, body: revised.text, ticketId: mail.ticketId, draftVersion: revised.draftVersion }})
    }}
}}
function processMailbox() {{
    const fetched = imapEmailFetch()
    for (const mail of forEach({{ array: fetched.mails }})) {{
        processOneMail({{ mail: mail }})
    }}
}}
eventsSimple() {{
    processMailbox()
}}
"#
        )
    }

    #[test]
    fn function_layered_approval_workflow_produces_no_false_notes() {
        let contract = derive_request_acceptance_contract(LAYERED_APPROVAL_REQUEST);
        let ast = flow_like_ast::parse(&layered_approval_flowscript(true))
            .expect("layered source parses");
        let diagnostics =
            flowscript_acceptance_diagnostics(&contract, &ast, &approval_flowscript_catalog());
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn function_layered_approval_with_reviewer_passed_as_argument_produces_no_false_notes() {
        let source = r#"function sendFinal(mail: object, draft: object) {
    smtpEmailSend({ to: mail.requesterEmail, body: draft.text, ticketId: mail.ticketId, draftVersion: draft.draftVersion })
}
function processOneMail(mail: object, approver: string) {
    const draft = aiModelGenerate()
    smtpEmailSend({ to: approver, body: draft.text, ticketId: mail.ticketId, draftVersion: draft.draftVersion })
    if (mail.incomingSender == approver && mail.reviewerApprovalDecision == "approved" && mail.ticketId == draft.ticketId && mail.draftVersion == draft.draftVersion) {
        sendFinal({ mail: mail, draft: draft })
    } else {
        const revised = aiModelGenerate({ reviewerFeedback: mail.reviewerChangeFeedback })
        smtpEmailSend({ to: approver, body: revised.text, ticketId: mail.ticketId, draftVersion: revised.draftVersion })
    }
}
function processMailbox() {
    const fetched = imapEmailFetch()
    for (const mail of forEach({ array: fetched.mails })) {
        processOneMail({ mail: mail, approver: "example@example.com" })
    }
}
eventsSimple() {
    processMailbox()
}
"#;
        let contract = derive_request_acceptance_contract(LAYERED_APPROVAL_REQUEST);
        let ast = flow_like_ast::parse(source).expect("layered source parses");
        let diagnostics =
            flowscript_acceptance_diagnostics(&contract, &ast, &approval_flowscript_catalog());
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn function_layered_approval_missing_sender_check_still_flags_the_sender() {
        let contract = derive_request_acceptance_contract(LAYERED_APPROVAL_REQUEST);
        let ast = flow_like_ast::parse(&layered_approval_flowscript(false))
            .expect("layered source parses");
        let diagnostics =
            flowscript_acceptance_diagnostics(&contract, &ast, &approval_flowscript_catalog());
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.code == FlowScriptDiagnosticCode::FsRequestApprovalInvalid
                    && diagnostic.message.contains("inbound sender")
            }),
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn flowscript_patch_is_unique_and_revision_cas_safe() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let source = valid_flowscript("hello");
        let written = store.write_flowscript(
            &board,
            &flowscript_catalog(),
            WriteFlowScriptArgs {
                draft_id: "source-cas".to_string(),
                replace_existing: false,
                mode: FlowIrDraftMode::Additive,
                source: source.clone(),
                allow_scope_reduction: false,
            },
        );
        assert_eq!(written.revision, Some(0));
        assert_eq!(written.source.as_deref(), Some(source.as_str()));

        let patched = store.patch_flowscript(
            &board,
            &flowscript_catalog(),
            PatchFlowScriptArgs {
                draft_id: "source-cas".to_string(),
                expected_revision: 0,
                old_text: "hello".to_string(),
                new_text: "goodbye".to_string(),
                allow_scope_reduction: false,
            },
        );
        assert_eq!(patched.revision, Some(1));
        assert!(patched.source.as_deref().unwrap().contains("goodbye"));

        let stale = store.patch_flowscript(
            &board,
            &flowscript_catalog(),
            PatchFlowScriptArgs {
                draft_id: "source-cas".to_string(),
                expected_revision: 0,
                old_text: "goodbye".to_string(),
                new_text: "replayed".to_string(),
                allow_scope_reduction: false,
            },
        );
        assert_eq!(stale.code.as_deref(), Some("FLOWSCRIPT_REVISION_CONFLICT"));
        assert_eq!(stale.revision, Some(1));

        let ambiguous = store.patch_flowscript(
            &board,
            &flowscript_catalog(),
            PatchFlowScriptArgs {
                draft_id: "source-cas".to_string(),
                expected_revision: 1,
                old_text: "e".to_string(),
                new_text: "E".to_string(),
                allow_scope_reduction: false,
            },
        );
        assert_eq!(
            ambiguous.code.as_deref(),
            Some("FLOWSCRIPT_PATCH_NOT_UNIQUE")
        );
        assert_eq!(ambiguous.revision, Some(1));
    }

    #[test]
    fn flowscript_check_commit_retains_exact_batch_in_shared_claim_store() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let source = valid_flowscript("hello");
        assert_eq!(
            store
                .write_flowscript(
                    &board,
                    &flowscript_catalog(),
                    WriteFlowScriptArgs {
                        draft_id: "source-claim".to_string(),
                        replace_existing: false,
                        mode: FlowIrDraftMode::Additive,
                        source: source.clone(),
                        allow_scope_reduction: false,
                    },
                )
                .revision,
            Some(0)
        );
        let checked = store.check_flowscript(
            &board,
            &flowscript_catalog(),
            CheckFlowScriptArgs {
                draft_id: "source-claim".to_string(),
                expected_revision: 0,
            },
        );
        assert_eq!(checked.status, "valid", "{checked:#?}");
        let exact_checked = store
            .source_drafts
            .lock()
            .unwrap()
            .get("source-claim")
            .unwrap()
            .checked
            .as_ref()
            .unwrap()
            .commands
            .clone();
        let queued = store.commit_flowscript(
            &board,
            &flowscript_catalog(),
            CommitFlowScriptArgs {
                draft_id: "source-claim".to_string(),
                expected_revision: 0,
                allow_deletions: false,
                remove_node_ids: Vec::new(),
                remove_variable_ids: Vec::new(),
                remove_layer_ids: Vec::new(),
                remove_comment_ids: Vec::new(),
            },
        );
        assert_eq!(queued.status, "queued", "{queued:#?}");
        assert_eq!(
            serde_json::to_value(&queued.commands).unwrap(),
            serde_json::to_value(&exact_checked).unwrap()
        );
        let token = store
            .latest_pending_commit_token(&board.id)
            .expect("source commit returns a standard pending claim");
        assert_eq!(token.draft_id, "source-claim");
        assert!(!token.requires_destructive_approval);
        let retained = store
            .pending_commands_if_current(
                &board,
                &token.draft_id,
                token.revision,
                &token.base_fingerprint,
                &token.claim_id,
            )
            .expect("source batch resolves through shared claim API");
        assert_eq!(
            serde_json::to_value(retained).unwrap(),
            serde_json::to_value(exact_checked).unwrap()
        );
        assert!(store.release_commit_if_matches(
            &token.draft_id,
            token.revision,
            &token.base_fingerprint,
            &token.claim_id,
        ));
    }

    #[test]
    fn typed_request_mismatch_envelopes_hide_foreign_coordinates() {
        let denied = draft_request_mismatch(
            "foreign-draft",
            17,
            "IR_DRAFT_REQUEST_IDENTITY_MISMATCH",
            "This typed draft belongs to another request.",
        );
        let serialized = serde_json::to_value(&denied).unwrap();
        assert!(serialized.get("draft_id").is_none());
        assert!(serialized.get("revision").is_none());

        let response = draft_request_mismatch_response(denied);
        assert_eq!(response.status, "request_identity_mismatch");
        assert!(response.draft_id.is_none());
        assert!(response.revision.is_none());
        assert!(response.base_fingerprint.is_none());
        assert!(response.flowscript.is_none());
        assert!(response.commands.is_empty());
    }

    #[test]
    fn flowscript_catalog_fingerprint_is_order_independent_and_contract_sensitive() {
        let catalog = flowscript_catalog();
        let baseline = flowscript_catalog_fingerprint(&catalog);

        let mut reordered = catalog.clone();
        reordered.reverse();
        assert_eq!(flowscript_catalog_fingerprint(&reordered), baseline);

        let mut changed_type = catalog.clone();
        changed_type[1].inputs[1].data_type = "Struct".to_string();
        assert_ne!(flowscript_catalog_fingerprint(&changed_type), baseline);

        let mut changed_schema = catalog;
        changed_schema[1].inputs[1].schema = Some(r#"{"type":"string"}"#.to_string());
        assert_ne!(flowscript_catalog_fingerprint(&changed_schema), baseline);
    }

    #[test]
    fn flowscript_catalog_fingerprint_ignores_nonsemantic_pin_and_companion_metadata() {
        let catalog = flowscript_catalog();
        let baseline = flowscript_catalog_fingerprint(&catalog);
        let mut presentation_change = catalog;
        presentation_change[0].companion_nodes = vec!["some_search_hint".to_string()];
        presentation_change[1].inputs[0].friendly_name = "New pin label".to_string();
        presentation_change[1].inputs[0].description = "New pin documentation".to_string();

        assert_eq!(
            flowscript_catalog_fingerprint(&presentation_change),
            baseline
        );
    }

    #[test]
    fn flowscript_catalog_fingerprint_tracks_acceptance_semantics() {
        let catalog = flowscript_catalog();
        let baseline = flowscript_catalog_fingerprint(&catalog);

        let mutations: [fn(&mut NodeMetadata); 4] = [
            |metadata: &mut NodeMetadata| metadata.friendly_name = "Send Email".to_string(),
            |metadata: &mut NodeMetadata| {
                metadata.description = "Publishes a support reply".to_string()
            },
            |metadata: &mut NodeMetadata| metadata.category = Some("support/email".to_string()),
            |metadata: &mut NodeMetadata| {
                metadata.capability_tags = vec!["send".to_string(), "email".to_string()]
            },
        ];
        for mutate in mutations {
            let mut changed = catalog.clone();
            mutate(&mut changed[0]);
            assert_ne!(flowscript_catalog_fingerprint(&changed), baseline);
        }
    }

    #[test]
    fn flowscript_commit_rejects_commands_checked_against_a_stale_catalog() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let catalog = flowscript_catalog();
        let draft_id = "source-stale-catalog";
        let written = store.write_flowscript(
            &board,
            &catalog,
            WriteFlowScriptArgs {
                draft_id: draft_id.to_string(),
                replace_existing: false,
                mode: FlowIrDraftMode::Additive,
                source: valid_flowscript("hello"),
                allow_scope_reduction: false,
            },
        );
        assert_eq!(written.revision, Some(0), "{written:#?}");
        let checked = store.check_flowscript(
            &board,
            &catalog,
            CheckFlowScriptArgs {
                draft_id: draft_id.to_string(),
                expected_revision: 0,
            },
        );
        assert_eq!(checked.status, "valid", "{checked:#?}");

        let mut changed_catalog = catalog;
        changed_catalog[1].outputs.push(pin("status", "String"));
        let commit_args = CommitFlowScriptArgs {
            draft_id: draft_id.to_string(),
            expected_revision: 0,
            allow_deletions: false,
            remove_node_ids: Vec::new(),
            remove_variable_ids: Vec::new(),
            remove_layer_ids: Vec::new(),
            remove_comment_ids: Vec::new(),
        };
        let refused = store.commit_flowscript(&board, &changed_catalog, commit_args.clone());
        assert_eq!(
            refused.code.as_deref(),
            Some("FLOWSCRIPT_CATALOG_REVISION_CONFLICT"),
            "{refused:#?}"
        );
        assert!(refused.commands.is_empty());
        assert!(store.latest_pending_commit_token(&board.id).is_none());

        let rechecked = store.check_flowscript(
            &board,
            &changed_catalog,
            CheckFlowScriptArgs {
                draft_id: draft_id.to_string(),
                expected_revision: 0,
            },
        );
        assert_eq!(rechecked.status, "valid", "{rechecked:#?}");
        let queued = store.commit_flowscript(&board, &changed_catalog, commit_args);
        assert_eq!(queued.status, "queued", "{queued:#?}");
        assert!(!queued.commands.is_empty());
    }

    #[test]
    fn pending_flowscript_delivery_redelivers_the_exact_claim_without_consuming_it() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let binding =
            store.bind_request_acceptance_contract(&board.id, "Log the customer message.");
        let source = valid_flowscript("customer");
        let written = store.write_flowscript_with_acceptance_binding(
            &board,
            &flowscript_catalog(),
            WriteFlowScriptArgs {
                draft_id: "source-redelivery".to_string(),
                replace_existing: false,
                mode: FlowIrDraftMode::Additive,
                source: source.clone(),
                allow_scope_reduction: false,
            },
            &binding,
        );
        assert_eq!(written.revision, Some(0), "{written:#?}");
        let checked = store.check_flowscript_with_acceptance_binding(
            &board,
            &flowscript_catalog(),
            CheckFlowScriptArgs {
                draft_id: "source-redelivery".to_string(),
                expected_revision: 0,
            },
            &binding,
        );
        assert_eq!(checked.status, "valid", "{checked:#?}");
        let queued = store.commit_flowscript_with_acceptance_binding(
            &board,
            &flowscript_catalog(),
            CommitFlowScriptArgs {
                draft_id: "source-redelivery".to_string(),
                expected_revision: 0,
                allow_deletions: false,
                remove_node_ids: Vec::new(),
                remove_variable_ids: Vec::new(),
                remove_layer_ids: Vec::new(),
                remove_comment_ids: Vec::new(),
            },
            &binding,
        );
        assert_eq!(queued.status, "queued", "{queued:#?}");
        assert!(!queued.commands.is_empty());
        let retained_access_sequence = store
            .source_drafts
            .lock()
            .unwrap()
            .get("source-redelivery")
            .unwrap()
            .access_sequence;

        let first = store
            .pending_flowscript_delivery_for_binding(&board, &binding)
            .expect("exact request can recover its current pending review");
        assert_eq!(first.source, source);
        assert!(!first.stale_board);
        assert_eq!(
            serde_json::to_value(&first.commands).unwrap(),
            serde_json::to_value(&queued.commands).unwrap()
        );
        let original_token = store
            .latest_pending_commit_token(&board.id)
            .expect("commit retained one pending token");
        assert_eq!(first.token, original_token);

        // Merely observing or dropping a recovery payload must not claim, rotate, or release it.
        let expected_token = first.token.clone();
        drop(first);
        let repeated = store
            .pending_flowscript_delivery_for_binding(&board, &binding)
            .expect("an interrupted redelivery remains retryable");
        assert!(!repeated.stale_board);
        assert_eq!(repeated.token, expected_token);
        assert_eq!(
            serde_json::to_value(&repeated.commands).unwrap(),
            serde_json::to_value(&queued.commands).unwrap()
        );
        assert!(store.pending_commit_matches(
            &expected_token.draft_id,
            expected_token.revision,
            &expected_token.base_fingerprint,
            &expected_token.claim_id,
        ));

        let unrelated = store.bind_request_acceptance_contract(
            &board.id,
            "Build an unrelated database cleanup workflow.",
        );
        assert!(
            store
                .pending_flowscript_delivery_for_binding(&board, &unrelated)
                .is_none()
        );

        let mut advanced = board.clone();
        let mut marker = Variable::new("revisionMarker", VariableType::String, ValueType::Normal);
        marker.id = "redelivery-revision-marker".to_string();
        advanced.variables.insert(marker.id.clone(), marker);
        let stale = store
            .pending_flowscript_delivery_for_binding(&advanced, &binding)
            .expect("an exact stale review remains recoverable for explicit dismissal");
        assert!(stale.stale_board);
        assert!(stale.commands.is_empty());
        assert_eq!(stale.source, source);
        assert_eq!(stale.token, expected_token);
        assert!(store.pending_commit_matches(
            &expected_token.draft_id,
            expected_token.revision,
            &expected_token.base_fingerprint,
            &expected_token.claim_id,
        ));
        assert_eq!(
            store
                .source_drafts
                .lock()
                .unwrap()
                .get("source-redelivery")
                .unwrap()
                .access_sequence,
            retained_access_sequence,
            "redelivery inspection must not mutate draft recency or claim state"
        );
    }

    #[test]
    fn flowscript_recovery_requires_exact_request_identity_and_retains_source() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let original_request = "Build a log workflow for the customer message.";
        let binding = store.bind_request_acceptance_contract(&board.id, original_request);
        let source = valid_flowscript("customer");
        let written = store.write_flowscript_with_acceptance_binding(
            &board,
            &flowscript_catalog(),
            WriteFlowScriptArgs {
                draft_id: "source-recovery".to_string(),
                replace_existing: false,
                mode: FlowIrDraftMode::Additive,
                source: source.clone(),
                allow_scope_reduction: false,
            },
            &binding,
        );
        assert_eq!(written.revision, Some(0));

        let resumed = store.editable_flowscript_draft_recovery(&board, original_request);
        assert!(resumed.auto_resume);
        let context = resumed.exact_match.expect("exact request resumes source");
        assert_eq!(context.source.as_deref(), Some(source.as_str()));
        assert_eq!(context.revision, 0);

        let unrelated_binding = store.bind_request_acceptance_contract(
            &board.id,
            "Build an unrelated database cleanup workflow.",
        );
        let denied_responses = [
            store.write_flowscript_with_acceptance_binding(
                &board,
                &flowscript_catalog(),
                WriteFlowScriptArgs {
                    draft_id: "source-recovery".to_string(),
                    replace_existing: true,
                    mode: FlowIrDraftMode::Additive,
                    source: valid_flowscript("attacker"),
                    allow_scope_reduction: false,
                },
                &unrelated_binding,
            ),
            store.patch_flowscript_with_acceptance_binding(
                &board,
                &flowscript_catalog(),
                PatchFlowScriptArgs {
                    draft_id: "source-recovery".to_string(),
                    expected_revision: 0,
                    old_text: "customer".to_string(),
                    new_text: "attacker".to_string(),
                    allow_scope_reduction: false,
                },
                &unrelated_binding,
            ),
            store.check_flowscript_with_acceptance_binding(
                &board,
                &flowscript_catalog(),
                CheckFlowScriptArgs {
                    draft_id: "source-recovery".to_string(),
                    expected_revision: 0,
                },
                &unrelated_binding,
            ),
            store.commit_flowscript_with_acceptance_binding(
                &board,
                &flowscript_catalog(),
                CommitFlowScriptArgs {
                    draft_id: "source-recovery".to_string(),
                    expected_revision: 0,
                    allow_deletions: false,
                    remove_node_ids: Vec::new(),
                    remove_variable_ids: Vec::new(),
                    remove_layer_ids: Vec::new(),
                    remove_comment_ids: Vec::new(),
                },
                &unrelated_binding,
            ),
        ];
        for denied in denied_responses {
            assert_eq!(
                denied.code.as_deref(),
                Some("FLOWSCRIPT_DRAFT_REQUEST_IDENTITY_MISMATCH")
            );
            assert_eq!(denied.status, "request_identity_mismatch");
            assert!(denied.draft_id.is_none());
            assert!(denied.revision.is_none());
            assert!(denied.base_fingerprint.is_none());
            assert!(denied.source.is_none());
            assert!(denied.diagnostics.is_empty());
            assert!(denied.corrections.is_empty());
            assert!(denied.derived_command_count.is_none());
            assert_eq!(denied.queued_count, 0);
            assert!(denied.commands.is_empty());
            let serialized = serde_json::to_value(&denied).unwrap();
            assert!(serialized.get("draft_id").is_none());
            assert!(serialized.get("revision").is_none());
            assert!(serialized.get("base_fingerprint").is_none());
            assert!(serialized.get("source").is_none());
        }

        let missing = store.patch_flowscript_with_acceptance_binding(
            &board,
            &flowscript_catalog(),
            PatchFlowScriptArgs {
                draft_id: "not-a-retained-draft".to_string(),
                expected_revision: 0,
                old_text: "customer".to_string(),
                new_text: "attacker".to_string(),
                allow_scope_reduction: false,
            },
            &unrelated_binding,
        );
        assert_eq!(missing.code.as_deref(), Some("FLOWSCRIPT_DRAFT_MISSING"));
        assert!(missing.draft_id.is_none());
        assert!(missing.revision.is_none());
        assert!(missing.source.is_none());

        let retained = store.source_drafts.lock().unwrap();
        let original = retained.get("source-recovery").unwrap();
        assert_eq!(original.revision, 0);
        assert_eq!(original.source, source);
        drop(retained);

        let conflicting = store.editable_flowscript_draft_recovery(
            &board,
            "Build an unrelated database cleanup workflow.",
        );
        assert!(!conflicting.auto_resume);
        assert_eq!(
            conflicting.status,
            FlowIrDraftRecoveryStatus::RequestMismatch
        );
        assert!(conflicting.exact_match.is_none());
        assert!(conflicting.conflicting_draft.is_none());
        let serialized = serde_json::to_string(&conflicting).unwrap();
        assert!(!serialized.contains("source-recovery"));
        assert!(!serialized.contains(&source));

        let separate = store.write_flowscript_with_acceptance_binding(
            &board,
            &flowscript_catalog(),
            WriteFlowScriptArgs {
                draft_id: "unrelated-recovery".to_string(),
                replace_existing: false,
                mode: FlowIrDraftMode::Additive,
                source: valid_flowscript("unrelated"),
                allow_scope_reduction: false,
            },
            &unrelated_binding,
        );
        assert_eq!(separate.revision, Some(0));
        let unrelated = store.editable_flowscript_draft_recovery(
            &board,
            "Build an unrelated database cleanup workflow.",
        );
        assert!(unrelated.auto_resume);
        assert_eq!(
            unrelated
                .exact_match
                .as_ref()
                .map(|draft| draft.draft_id.as_str()),
            Some("unrelated-recovery")
        );
        let original = store.editable_flowscript_draft_recovery(&board, original_request);
        assert_eq!(
            original
                .exact_match
                .as_ref()
                .and_then(|draft| draft.source.as_deref()),
            Some(source.as_str())
        );
    }

    #[test]
    fn flowscript_check_rejects_a_stale_board_fingerprint() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let source = valid_flowscript("hello");
        store.write_flowscript(
            &board,
            &flowscript_catalog(),
            WriteFlowScriptArgs {
                draft_id: "source-stale".to_string(),
                replace_existing: false,
                mode: FlowIrDraftMode::Additive,
                source: source.clone(),
                allow_scope_reduction: false,
            },
        );
        let mut advanced = board.clone();
        let mut variable = Variable::new("revisionMarker", VariableType::String, ValueType::Normal);
        variable.id = "revision-marker".to_string();
        advanced.variables.insert(variable.id.clone(), variable);
        let checked = store.check_flowscript(
            &advanced,
            &flowscript_catalog(),
            CheckFlowScriptArgs {
                draft_id: "source-stale".to_string(),
                expected_revision: 0,
            },
        );
        assert_eq!(
            checked.code.as_deref(),
            Some("FLOWSCRIPT_BASE_REVISION_CONFLICT")
        );
        assert_eq!(checked.source.as_deref(), Some(source.as_str()));

        let patched = store.patch_flowscript(
            &advanced,
            &flowscript_catalog(),
            PatchFlowScriptArgs {
                draft_id: "source-stale".to_string(),
                expected_revision: 0,
                old_text: "hello".to_string(),
                new_text: "changed".to_string(),
                allow_scope_reduction: false,
            },
        );
        assert_eq!(
            patched.code.as_deref(),
            Some("FLOWSCRIPT_BASE_REVISION_CONFLICT")
        );
        let replaced = store.write_flowscript(
            &advanced,
            &flowscript_catalog(),
            WriteFlowScriptArgs {
                draft_id: "source-stale".to_string(),
                replace_existing: true,
                mode: FlowIrDraftMode::Additive,
                source: valid_flowscript("replacement"),
                allow_scope_reduction: false,
            },
        );
        assert_eq!(
            replaced.code.as_deref(),
            Some("FLOWSCRIPT_BASE_REVISION_CONFLICT")
        );
        let retained_drafts = store.source_drafts.lock().unwrap();
        let retained = retained_drafts.get("source-stale").unwrap();
        assert_eq!(retained.revision, 0);
        assert_eq!(retained.source, source);
        drop(retained_drafts);

        let recovery = store.editable_flowscript_draft_recovery_for_identity(
            &advanced,
            &FlowIrRequestIdentity::unbound(),
        );
        assert!(!recovery.auto_resume);
        assert!(
            recovery
                .exact_match
                .as_ref()
                .is_some_and(|context| context.stale_board)
        );
        assert_eq!(
            recovery.next_actions,
            ["start_new_draft_from_current_board"]
        );
    }

    #[test]
    fn base_revision_conflict_reopens_the_request_binding_for_a_new_draft() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let request = "Send a Slack notification.";
        let binding = store.bind_request_acceptance_contract(&board.id, request);
        let written = store.write_flowscript_with_acceptance_binding(
            &board,
            &flowscript_catalog(),
            WriteFlowScriptArgs {
                draft_id: "conflict-first".to_string(),
                replace_existing: false,
                mode: FlowIrDraftMode::Additive,
                source: valid_flowscript("hello"),
                allow_scope_reduction: false,
            },
            &binding,
        );
        assert_eq!(written.revision, Some(0), "{written:#?}");

        let mut advanced = board.clone();
        let mut variable = Variable::new("conflictMarker", VariableType::String, ValueType::Normal);
        variable.id = "conflict-marker".to_string();
        advanced.variables.insert(variable.id.clone(), variable);
        let checked = store.check_flowscript_with_acceptance_binding(
            &advanced,
            &flowscript_catalog(),
            CheckFlowScriptArgs {
                draft_id: "conflict-first".to_string(),
                expected_revision: 0,
            },
            &binding,
        );
        assert_eq!(
            checked.code.as_deref(),
            Some("FLOWSCRIPT_BASE_REVISION_CONFLICT")
        );

        // The re-opened contract still belongs to its immutable request: a binding carrying a
        // different request identity must not claim it.
        let forged = FlowIrAcceptanceBinding {
            id: binding.id.clone(),
            board_id: board.id.clone(),
            criterion_count: binding.criterion_count,
            request_identity: FlowIrRequestIdentity::from_raw_request(
                "Build an unrelated database cleanup workflow.",
            ),
        };
        let denied = store.write_flowscript_with_acceptance_binding(
            &advanced,
            &flowscript_catalog(),
            WriteFlowScriptArgs {
                draft_id: "conflict-foreign".to_string(),
                replace_existing: false,
                mode: FlowIrDraftMode::Additive,
                source: valid_flowscript("hello"),
                allow_scope_reduction: false,
            },
            &forged,
        );
        assert_eq!(
            denied.code.as_deref(),
            Some("IR_ACCEPTANCE_BINDING_IDENTITY_MISMATCH"),
            "{denied:#?}"
        );

        let recovered = store.write_flowscript_with_acceptance_binding(
            &advanced,
            &flowscript_catalog(),
            WriteFlowScriptArgs {
                draft_id: "conflict-second".to_string(),
                replace_existing: false,
                mode: FlowIrDraftMode::Additive,
                source: valid_flowscript("hello"),
                allow_scope_reduction: false,
            },
            &binding,
        );
        assert_eq!(
            recovered.revision,
            Some(0),
            "a fresh draft under the same binding must succeed after the conflict: {recovered:#?}"
        );
        let drafts = store.source_drafts.lock().unwrap();
        let first = drafts.get("conflict-first").unwrap();
        let second = drafts.get("conflict-second").unwrap();
        assert_eq!(second.request_identity, first.request_identity);
        assert_eq!(
            second.request_acceptance_contract, first.request_acceptance_contract,
            "the recovered draft must carry the identical request contract"
        );
        assert!(!second.request_acceptance_contract.criteria.is_empty());
        drop(drafts);
        assert!(
            store
                .request_acceptance_contracts
                .lock()
                .unwrap()
                .is_empty(),
            "the successful recovery write consumes the re-opened contract again"
        );
    }

    #[test]
    fn typed_base_revision_conflict_reopens_the_request_binding_for_a_new_draft() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let request = "Send a Slack notification.";
        let binding = store.bind_request_acceptance_contract(&board.id, request);
        let started = store.begin_with_acceptance_binding(
            &board,
            &catalog(),
            BeginFlowIrDraftArgs {
                draft_id: "typed-conflict-first".to_string(),
                replace_existing: false,
                expected_modules: vec!["eventsSimple".to_string()],
                capability_plan: capability_plan(),
                mode: FlowIrDraftMode::Additive,
                program: program("before"),
            },
            &binding,
        );
        assert_eq!(started.status, "draft_started", "{started:#?}");

        let mut advanced = board.clone();
        let mut variable = Variable::new("conflictMarker", VariableType::String, ValueType::Normal);
        variable.id = "typed-conflict-marker".to_string();
        advanced.variables.insert(variable.id.clone(), variable);
        let conflicted = store.commit_with_acceptance_binding(
            &advanced,
            &catalog(),
            CommitFlowIrDraftArgs {
                draft_id: "typed-conflict-first".to_string(),
                expected_revision: 0,
                allow_deletions: false,
                remove_node_ids: Vec::new(),
                remove_variable_ids: Vec::new(),
                remove_layer_ids: Vec::new(),
                remove_comment_ids: Vec::new(),
                use_best_candidate: false,
            },
            &binding,
        );
        assert_eq!(
            conflicted.code.as_deref(),
            Some("IR_BASE_REVISION_CONFLICT"),
            "{conflicted:#?}"
        );

        // The re-opened contract still belongs to its immutable request: a binding carrying a
        // different request identity must not claim it.
        let forged = FlowIrAcceptanceBinding {
            id: binding.id.clone(),
            board_id: board.id.clone(),
            criterion_count: binding.criterion_count,
            request_identity: FlowIrRequestIdentity::from_raw_request(
                "Build an unrelated database cleanup workflow.",
            ),
        };
        let denied = store.begin_with_acceptance_binding(
            &advanced,
            &catalog(),
            BeginFlowIrDraftArgs {
                draft_id: "typed-conflict-foreign".to_string(),
                replace_existing: false,
                expected_modules: vec!["eventsSimple".to_string()],
                capability_plan: capability_plan(),
                mode: FlowIrDraftMode::Additive,
                program: program("before"),
            },
            &forged,
        );
        assert_eq!(
            denied.code.as_deref(),
            Some("IR_ACCEPTANCE_BINDING_IDENTITY_MISMATCH"),
            "{denied:#?}"
        );

        let recovered = store.begin_with_acceptance_binding(
            &advanced,
            &catalog(),
            BeginFlowIrDraftArgs {
                draft_id: "typed-conflict-second".to_string(),
                replace_existing: false,
                expected_modules: vec!["eventsSimple".to_string()],
                capability_plan: capability_plan(),
                mode: FlowIrDraftMode::Additive,
                program: program("before"),
            },
            &binding,
        );
        assert_eq!(
            recovered.status, "draft_started",
            "a fresh typed draft under the same binding must succeed after the conflict: {recovered:#?}"
        );
        let drafts = store.drafts.lock().unwrap();
        let first = drafts.get("typed-conflict-first").unwrap();
        let second = drafts.get("typed-conflict-second").unwrap();
        assert_eq!(second.request_identity, first.request_identity);
        assert_eq!(
            second.request_acceptance_contract, first.request_acceptance_contract,
            "the recovered typed draft must carry the identical request contract"
        );
        assert!(!second.request_acceptance_contract.criteria.is_empty());
        drop(drafts);
        assert!(
            store
                .request_acceptance_contracts
                .lock()
                .unwrap()
                .is_empty(),
            "the successful recovery begin consumes the re-opened contract again"
        );
    }

    #[test]
    fn typed_missing_draft_releases_the_request_claim_for_a_new_draft() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        store.write_flowscript(
            &board,
            &flowscript_catalog(),
            WriteFlowScriptArgs {
                draft_id: "occupied".to_string(),
                replace_existing: false,
                mode: FlowIrDraftMode::Additive,
                source: valid_flowscript("hello"),
                allow_scope_reduction: false,
            },
        );
        let binding =
            store.bind_request_acceptance_contract(&board.id, "Send a Slack notification.");
        let begin_args = |draft_id: &str| BeginFlowIrDraftArgs {
            draft_id: draft_id.to_string(),
            replace_existing: false,
            expected_modules: vec!["eventsSimple".to_string()],
            capability_plan: capability_plan(),
            mode: FlowIrDraftMode::Additive,
            program: program("before"),
        };
        // The claim succeeds before the id collision is detected, so the failed begin leaves the
        // pending contract attached to a typed draft that was never stored.
        let collided = store.begin_with_acceptance_binding(
            &board,
            &catalog(),
            begin_args("occupied"),
            &binding,
        );
        assert_eq!(
            collided.code.as_deref(),
            Some("IR_DRAFT_ID_COLLISION"),
            "{collided:#?}"
        );
        let blocked = store.begin_with_acceptance_binding(
            &board,
            &catalog(),
            begin_args("typed-recovery"),
            &binding,
        );
        assert_eq!(
            blocked.code.as_deref(),
            Some("IR_ACCEPTANCE_BINDING_ALREADY_CLAIMED"),
            "{blocked:#?}"
        );
        // Observing the missing typed draft releases exactly that stale claim.
        let missing = store.validate_with_acceptance_binding(
            &board,
            &catalog(),
            ValidateFlowIrDraftArgs {
                draft_id: "occupied".to_string(),
                include_header: false,
                modules: Vec::new(),
            },
            &binding,
        );
        assert_eq!(missing.code.as_deref(), Some("IR_DRAFT_MISSING"));
        let recovered = store.begin_with_acceptance_binding(
            &board,
            &catalog(),
            begin_args("typed-recovery"),
            &binding,
        );
        assert_eq!(recovered.status, "draft_started", "{recovered:#?}");
    }

    #[test]
    fn flowscript_patch_does_not_replace_a_substantial_draft_with_a_smoke_test() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let substantial = r#"function collect() {
    logInfo({ message: "one" })
    logInfo({ message: "two" })
    logInfo({ message: "three" })
}
function enrich() {
    logInfo({ message: "four" })
    logInfo({ message: "five" })
    logInfo({ message: "six" })
}
eventsSimple() {
    collect()
    enrich()
}
"#;
        store.write_flowscript(
            &board,
            &flowscript_catalog(),
            WriteFlowScriptArgs {
                draft_id: "source-regression".to_string(),
                replace_existing: false,
                mode: FlowIrDraftMode::Additive,
                source: substantial.to_string(),
                allow_scope_reduction: false,
            },
        );
        let tiny = valid_flowscript("smoke test");
        let blocked = store.patch_flowscript(
            &board,
            &flowscript_catalog(),
            PatchFlowScriptArgs {
                draft_id: "source-regression".to_string(),
                expected_revision: 0,
                old_text: substantial.to_string(),
                new_text: tiny,
                allow_scope_reduction: false,
            },
        );
        assert_eq!(
            blocked.code.as_deref(),
            Some("FLOWSCRIPT_CANDIDATE_REGRESSION")
        );
        assert_eq!(blocked.revision, Some(0));
        assert_eq!(blocked.source.as_deref(), Some(substantial));
    }

    #[test]
    fn malformed_expansion_never_becomes_the_flowscript_regression_baseline() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let parseable = r#"eventsSimple() {
    logInfo({ message: "one" })
    logInfo({ message: "two" })
    logInfo({ message: "three" })
    logInfo({ message: "four" })
    logInfo({ message: "five" })
    logInfo({ message: "six" })
}
"#;
        // Four event-sized slices make this lexically much more complete than `parseable`, but the
        // final missing brace keeps it syntactically invalid. If retained as the best candidate,
        // its expanded event scope would make restoring the complete parseable draft look like an
        // unauthorized scope collapse.
        let malformed_tail = r#"eventsSimple() {
    logInfo({ message: "one" })
    logInfo({ message: "two" })
    logInfo({ message: "three" })
    logInfo({ message: "four" })
    logInfo({ message: "five" })
    logInfo({ message: "six" })
"#;
        let malformed = format!("{parseable}{parseable}{parseable}{malformed_tail}");
        assert!(flow_like_ast::parse(parseable).is_ok());
        assert!(flow_like_ast::parse(&malformed).is_err());
        assert!(
            detect_flowscript_candidate_regression(
                &profile_flowscript_candidate(&malformed),
                &profile_flowscript_candidate(parseable),
            )
            .is_some(),
            "the regression must exercise the poisoned-baseline failure mode"
        );

        let written = store.write_flowscript(
            &board,
            &flowscript_catalog(),
            WriteFlowScriptArgs {
                draft_id: "source-malformed-expansion".to_string(),
                replace_existing: false,
                mode: FlowIrDraftMode::Additive,
                source: parseable.to_string(),
                allow_scope_reduction: false,
            },
        );
        assert_eq!(written.revision, Some(0));

        let expanded = store.patch_flowscript(
            &board,
            &flowscript_catalog(),
            PatchFlowScriptArgs {
                draft_id: "source-malformed-expansion".to_string(),
                expected_revision: 0,
                old_text: parseable.to_string(),
                new_text: malformed.clone(),
                allow_scope_reduction: false,
            },
        );
        assert_eq!(expanded.status, "validation_errors");
        assert_eq!(expanded.revision, Some(1));
        {
            let drafts = store.source_drafts.lock().unwrap();
            let retained = drafts.get("source-malformed-expansion").unwrap();
            assert!(retained.best_candidate.parse_valid);
            assert_eq!(retained.best_candidate.source, parseable);
        }

        let repaired = store.patch_flowscript(
            &board,
            &flowscript_catalog(),
            PatchFlowScriptArgs {
                draft_id: "source-malformed-expansion".to_string(),
                expected_revision: 1,
                old_text: malformed,
                new_text: parseable.to_string(),
                allow_scope_reduction: false,
            },
        );
        assert_ne!(
            repaired.code.as_deref(),
            Some("FLOWSCRIPT_CANDIDATE_REGRESSION")
        );
        assert_eq!(repaired.revision, Some(2));
        assert_eq!(repaired.source.as_deref(), Some(parseable));
    }

    #[test]
    fn acceptance_contract_is_conservative_about_compound_nouns_and_conjunctions() {
        let vague = derive_request_acceptance_contract(
            "Create a customer onboarding and account provisioning workflow.",
        );
        assert!(vague.criteria.is_empty());

        let explicit = derive_request_acceptance_contract(
            "Fetch customers and orders from the CRM, then send a summary email.",
        );
        assert_eq!(explicit.criteria.len(), 2);
        assert_eq!(explicit.criteria[0].actions, vec!["read"]);
        assert!(explicit.criteria[0].objects.is_empty());
        assert_eq!(explicit.criteria[1].actions, vec!["send"]);
        assert_eq!(explicit.criteria[1].objects, vec!["email"]);
    }

    #[test]
    fn explicit_noun_only_list_items_form_an_acceptance_contract() {
        let contract = derive_request_acceptance_contract(
            "Build this automation:\n- HTTP webhook ingestion\n- JSON validation\n- Slack notification",
        );
        assert_eq!(contract.criteria.len(), 3);
        assert!(contract.criteria[0].actions.is_empty());
        assert!(contract.criteria[0].objects.contains(&"http".to_string()));
        assert!(contract.criteria[2].objects.contains(&"slack".to_string()));
    }

    #[test]
    fn staged_begin_defers_host_acceptance_until_validate() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let binding = store.bind_request_acceptance_contract(
            &board.id,
            "Format the customer message, then send a Slack notification.",
        );
        assert_eq!(binding.criterion_count(), 2);
        let response = store.begin_with_acceptance_binding(
            &board,
            &acceptance_catalog(),
            BeginFlowIrDraftArgs {
                draft_id: "omitted-clause".to_string(),
                replace_existing: false,
                expected_modules: vec!["eventsSimple".to_string()],
                capability_plan: acceptance_capability_plan(false),
                mode: FlowIrDraftMode::Additive,
                program: program("customer message"),
            },
            &binding,
        );
        assert_eq!(response.status, "draft_started");
        assert!(response.diagnostics.is_empty());
        let validated = store.validate(
            &board,
            &acceptance_catalog(),
            ValidateFlowIrDraftArgs {
                draft_id: "omitted-clause".to_string(),
                include_header: false,
                modules: Vec::new(),
            },
        );
        assert!(validated.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "IR_REQUEST_ACCEPTANCE_CONTRACT_INCOMPLETE"
                && diagnostic.message.contains("Slack")
        }));
    }

    #[test]
    fn staged_upserts_do_not_run_global_reconcile_and_validate_runs_it_once() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let live_catalog = catalog();
        let started = store.begin(
            &board,
            &live_catalog,
            BeginFlowIrDraftArgs {
                draft_id: "staged-counter".to_string(),
                replace_existing: false,
                expected_modules: vec!["eventsSimple".to_string()],
                capability_plan: capability_plan(),
                mode: FlowIrDraftMode::Additive,
                program: program("message-0"),
            },
        );
        assert_eq!(started.status, "draft_started");
        assert_eq!(store.global_evaluation_count(), 0);

        for revision in 0..4_u64 {
            let replacement = program(&format!("message-{}", revision + 1))
                .modules
                .remove(0);
            let response = store.upsert_module(
                &board,
                &live_catalog,
                UpsertFlowIrModuleArgs {
                    draft_id: "staged-counter".to_string(),
                    expected_revision: revision,
                    allow_scope_reduction: false,
                    module: replacement,
                },
            );
            assert_eq!(response.revision, Some(revision + 1), "{response:#?}");
            assert_eq!(store.global_evaluation_count(), 0);
        }

        let _validated = store.validate(
            &board,
            &live_catalog,
            ValidateFlowIrDraftArgs {
                draft_id: "staged-counter".to_string(),
                include_header: false,
                modules: Vec::new(),
            },
        );
        assert_eq!(store.global_evaluation_count(), 1);
    }

    #[test]
    fn recovery_and_claim_queries_are_not_blocked_by_staged_evaluation() {
        let store = Arc::new(FlowIrDraftStore::new());
        let board = empty_board();
        let live_catalog = catalog();
        let started = store.begin(
            &board,
            &live_catalog,
            BeginFlowIrDraftArgs {
                draft_id: "unlocked-evaluation".to_string(),
                replace_existing: false,
                expected_modules: vec!["eventsSimple".to_string()],
                capability_plan: capability_plan(),
                mode: FlowIrDraftMode::Additive,
                program: program("before"),
            },
        );
        assert_eq!(started.status, "draft_started");

        let gate = store.pause_next_staged_evaluation();
        let mut replacement_program = program("after");
        let replacement = replacement_program.modules.remove(0);
        let worker_store = store.clone();
        let worker_board = board.clone();
        let worker_catalog = live_catalog.clone();
        let worker = thread::spawn(move || {
            worker_store.upsert_module(
                &worker_board,
                &worker_catalog,
                UpsertFlowIrModuleArgs {
                    draft_id: "unlocked-evaluation".to_string(),
                    expected_revision: 0,
                    allow_scope_reduction: false,
                    module: replacement,
                },
            )
        });
        if !gate.wait_until_entered(Duration::from_secs(2)) {
            gate.release();
            let _ = worker.join();
            panic!("staged evaluation never reached the test gate");
        }

        let recovery_store = store.clone();
        let recovery_board = board.clone();
        let recovery_catalog = live_catalog.clone();
        let (done_tx, done_rx) = mpsc::channel();
        let recovery = thread::spawn(move || {
            let context =
                recovery_store.latest_editable_draft_context(&recovery_board, &recovery_catalog);
            let claim_query_completed = !recovery_store.has_pending_commit();
            let _ = done_tx.send((context.is_some(), claim_query_completed));
        });
        let lookup_result = done_rx.recv_timeout(Duration::from_secs(1));
        gate.release();
        let response = worker.join().expect("upsert thread panicked");
        assert_eq!(
            lookup_result,
            Ok((true, true)),
            "draft recovery/claim lookup waited on the evaluation critical section"
        );
        assert_eq!(response.revision, Some(1));
        recovery.join().expect("recovery thread panicked");
    }

    #[test]
    fn email_notification_cannot_cover_a_slack_subject() {
        let contract = derive_request_acceptance_contract(
            "Format the customer message, then send a Slack notification.",
        );
        let mut catalog = acceptance_catalog();
        catalog.push(metadata("email_send", Vec::new(), Vec::new()));
        let email_only = FlowIrProgram {
            modules: vec![FlowIrModule::Event {
                name: "eventsSimple".to_string(),
                node_type: "events_simple".to_string(),
                params: Vec::new(),
                steps: vec![
                    program("customer message").modules.remove(0).steps()[0].clone(),
                    FlowIrStep::Node {
                        id: "notify".to_string(),
                        node_type: "email_send".to_string(),
                        args: Vec::new(),
                        continue_from: None,
                        exec_arms: Vec::new(),
                        anchor: None,
                    },
                ],
                anchor: None,
            }],
            ..Default::default()
        };
        let diagnostics = acceptance_contract_diagnostics(&contract, &email_only, &catalog);
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("Slack"));
    }

    #[test]
    fn acceptance_ignores_unreachable_helpers_but_accepts_called_helpers() {
        let contract = derive_request_acceptance_contract(
            "Format the customer message, then send a Slack notification.",
        );
        let FlowIrModule::Event { mut steps, .. } = program("customer message").modules.remove(0)
        else {
            unreachable!()
        };
        let helper = FlowIrModule::Function {
            name: "notifySlack".to_string(),
            params: Vec::new(),
            returns: Vec::new(),
            steps: acceptance_program().modules[0].steps()[1..].to_vec(),
            anchor: None,
        };
        let event = |steps| FlowIrModule::Event {
            name: "eventsSimple".to_string(),
            node_type: "events_simple".to_string(),
            params: Vec::new(),
            steps,
            anchor: None,
        };
        let unreachable = FlowIrProgram {
            modules: vec![helper.clone(), event(steps.clone())],
            ..Default::default()
        };
        assert_eq!(
            acceptance_contract_diagnostics(&contract, &unreachable, &acceptance_catalog()).len(),
            1
        );
        steps.push(FlowIrStep::CallFunction {
            id: "call_notify".to_string(),
            function: "notifySlack".to_string(),
            args: Vec::new(),
            anchor: None,
        });
        let called = FlowIrProgram {
            modules: vec![helper, event(steps)],
            ..Default::default()
        };
        assert!(
            acceptance_contract_diagnostics(&contract, &called, &acceptance_catalog()).is_empty()
        );
    }

    #[test]
    fn german_numbered_support_approval_loop_requires_real_correlated_control_flow() {
        let request = "Bau mir eine App:\n\
            1. IMAP-E-Mails abrufen -> Das Modell beantwortet die Supportanfrage\n\
            2. Eine Freigabemail an example@example.com senden\n\
            3. Bei Freigabe eine Kundenmail senden\n\
            4. Sonst den Entwurf anpassen und erneut eine Freigabe anfragen";
        let contract = derive_request_acceptance_contract(request);
        assert_eq!(
            contract
                .approval_loop
                .as_ref()
                .expect("approval loop derived")
                .reviewer_emails,
            ["example@example.com"]
        );
        assert_eq!(
            contract.criteria.len(),
            5,
            "derived criteria: {:#?}",
            contract.criteria
        );
        assert_eq!(contract.criteria[0].actions, vec!["read"]);
        assert_eq!(contract.criteria[1].actions, vec!["generate"]);
        // The review mail and customer mail clauses demand the same action on the same subject,
        // so they collapse into one criterion instead of duplicating it.
        assert_eq!(contract.criteria[2].actions, vec!["send"]);
        assert_eq!(contract.criteria[3].actions, vec!["update"]);
        assert_eq!(contract.criteria[4].actions, vec!["call"]);

        let mut catalog = vec![metadata("events_simple", Vec::new(), Vec::new())];
        catalog.extend([
            metadata("imap_email_fetch", Vec::new(), Vec::new()),
            metadata("ai_model_generate", Vec::new(), Vec::new()),
            metadata("smtp_email_send", Vec::new(), Vec::new()),
        ]);
        let complete = FlowIrProgram {
            modules: vec![FlowIrModule::Event {
                name: "eventsSimple".to_string(),
                node_type: "events_simple".to_string(),
                params: Vec::new(),
                steps: [
                    "imap_email_fetch",
                    "ai_model_generate",
                    "smtp_email_send",
                    "smtp_email_send",
                    "ai_model_generate",
                    "smtp_email_send",
                ]
                .into_iter()
                .enumerate()
                .map(|(index, node_type)| FlowIrStep::Node {
                    id: format!("step_{index}"),
                    node_type: node_type.to_string(),
                    args: Vec::new(),
                    continue_from: None,
                    exec_arms: Vec::new(),
                    anchor: None,
                })
                .collect(),
                anchor: None,
            }],
            ..Default::default()
        };
        let flat_diagnostics = acceptance_contract_diagnostics(&contract, &complete, &catalog);
        assert!(
            flat_diagnostics
                .iter()
                .any(|diagnostic| { diagnostic.code == "IR_REQUEST_APPROVAL_BRANCH_MISSING" })
        );

        fn reference(name: &str) -> FlowIrValue {
            FlowIrValue::Ref {
                name: name.to_string(),
            }
        }

        fn literal(value: &str) -> FlowIrValue {
            FlowIrValue::Literal {
                value: FlowIrLiteral::String(value.to_string()),
            }
        }

        fn node(id: &str, node_type: &str, args: Vec<FlowIrArg>) -> FlowIrStep {
            FlowIrStep::Node {
                id: id.to_string(),
                node_type: node_type.to_string(),
                args,
                continue_from: None,
                exec_arms: Vec::new(),
                anchor: None,
            }
        }

        fn mail(id: &str, recipient: FlowIrValue, include_correlation: bool) -> FlowIrStep {
            let mut args = vec![FlowIrArg {
                pin: "to".to_string(),
                occurrence: 0,
                value: recipient,
            }];
            if include_correlation {
                args.extend([
                    FlowIrArg {
                        pin: "ticket_id".to_string(),
                        occurrence: 0,
                        value: reference("ticket_id"),
                    },
                    FlowIrArg {
                        pin: "draft_version".to_string(),
                        occurrence: 0,
                        value: reference("draft_version"),
                    },
                ]);
            }
            node(id, "smtp_email_send", args)
        }

        fn nested_approval_program(
            outbound_reviewer: &str,
            validate_reviewer_sender: bool,
            consume_feedback: bool,
            reask_correlation: bool,
            literal_condition: bool,
        ) -> FlowIrProgram {
            let mut validation_args = vec![
                FlowIrArg {
                    pin: "incoming_sender".to_string(),
                    occurrence: 0,
                    value: reference("incoming_sender"),
                },
                FlowIrArg {
                    pin: "decision".to_string(),
                    occurrence: 0,
                    value: reference("reviewer_approval_decision"),
                },
                FlowIrArg {
                    pin: "ticket_id".to_string(),
                    occurrence: 0,
                    value: reference("ticket_id"),
                },
                FlowIrArg {
                    pin: "draft_version".to_string(),
                    occurrence: 0,
                    value: reference("draft_version"),
                },
            ];
            if validate_reviewer_sender {
                validation_args.push(FlowIrArg {
                    pin: "expected_reviewer".to_string(),
                    occurrence: 0,
                    value: literal("example@example.com"),
                });
            }
            let mut regenerate_args = Vec::new();
            if consume_feedback {
                regenerate_args.push(FlowIrArg {
                    pin: "reviewer_feedback".to_string(),
                    occurrence: 0,
                    value: reference("reviewer_change_feedback"),
                });
            }
            let condition = if literal_condition {
                FlowIrValue::Literal {
                    value: FlowIrLiteral::Boolean(true),
                }
            } else {
                FlowIrValue::Output {
                    step: "validate_reviewer_decision".to_string(),
                    pin: "approved_reviewer_decision".to_string(),
                    occurrence: 0,
                }
            };
            FlowIrProgram {
                modules: vec![FlowIrModule::Event {
                    name: "eventsSimple".to_string(),
                    node_type: "events_simple".to_string(),
                    params: Vec::new(),
                    steps: vec![
                        node("fetch", "imap_email_fetch", Vec::new()),
                        node("draft", "ai_model_generate", Vec::new()),
                        mail("initial_review", literal(outbound_reviewer), true),
                        node(
                            "validate_reviewer_decision",
                            "reviewer_decision_validate",
                            validation_args,
                        ),
                        FlowIrStep::If {
                            id: "route_approval".to_string(),
                            condition,
                            then_steps: vec![mail(
                                "send_customer",
                                reference("requester_email"),
                                true,
                            )],
                            else_steps: vec![
                                node("regenerate", "ai_model_generate", regenerate_args),
                                mail(
                                    "reask_reviewer",
                                    literal(outbound_reviewer),
                                    reask_correlation,
                                ),
                            ],
                            anchor: None,
                        },
                    ],
                    anchor: None,
                }],
                ..Default::default()
            }
        }

        catalog.push(metadata(
            "reviewer_decision_validate",
            Vec::new(),
            Vec::new(),
        ));
        let valid = nested_approval_program("example@example.com", true, true, true, false);
        assert!(
            acceptance_contract_diagnostics(&contract, &valid, &catalog).is_empty(),
            "valid diagnostics: {:#?}",
            acceptance_contract_diagnostics(&contract, &valid, &catalog)
        );

        let wrong_reviewer =
            nested_approval_program("attacker@example.com", true, true, true, false);
        assert!(
            acceptance_contract_diagnostics(&contract, &wrong_reviewer, &catalog)
                .iter()
                .any(|diagnostic| diagnostic.code == "IR_REQUEST_APPROVAL_REVIEWER_MISMATCH")
        );

        let arbitrary_sender =
            nested_approval_program("example@example.com", false, true, true, false);
        assert!(
            acceptance_contract_diagnostics(&contract, &arbitrary_sender, &catalog)
                .iter()
                .any(|diagnostic| diagnostic.code == "IR_REQUEST_APPROVAL_SENDER_UNVERIFIED")
        );

        let ignores_feedback =
            nested_approval_program("example@example.com", true, false, true, false);
        assert!(
            acceptance_contract_diagnostics(&contract, &ignores_feedback, &catalog)
                .iter()
                .any(|diagnostic| diagnostic.code == "IR_REQUEST_APPROVAL_REASK_INCOMPLETE")
        );

        let literal_decision =
            nested_approval_program("example@example.com", true, true, true, true);
        assert!(
            acceptance_contract_diagnostics(&contract, &literal_decision, &catalog)
                .iter()
                .any(|diagnostic| {
                    diagnostic.code == "IR_REQUEST_APPROVAL_CONDITION_UNCORRELATED"
                })
        );

        let missing_reask_correlation =
            nested_approval_program("example@example.com", true, true, false, false);
        assert!(
            acceptance_contract_diagnostics(&contract, &missing_reask_correlation, &catalog,)
                .iter()
                .any(|diagnostic| diagnostic.code == "IR_REQUEST_APPROVAL_CORRELATION_MISSING")
        );
    }

    #[test]
    fn page_action_approval_uses_distinct_ui_action_contract() {
        let prompt = "Human-in-the-Loop Freigabe auf einer Seite. \
            Page-Action approve erhält ticketId und draftReply. \
            Page-Action revise erhält ticketId und revisionFeedback.";
        let contract = derive_request_approval_loop_contract(prompt)
            .expect("the explicit approval loop should be detected");
        assert_eq!(contract.channel, RequestApprovalChannel::PageAction);
    }

    #[test]
    fn ui_action_approval_accepts_separate_approve_and_revise_entries() {
        fn reference(name: &str) -> FlowIrValue {
            FlowIrValue::Ref {
                name: name.to_string(),
            }
        }

        fn literal(value: &str) -> FlowIrValue {
            FlowIrValue::Literal {
                value: FlowIrLiteral::String(value.to_string()),
            }
        }

        fn arg(pin: &str, value: FlowIrValue) -> FlowIrArg {
            FlowIrArg {
                pin: pin.to_string(),
                occurrence: 0,
                value,
            }
        }

        fn node(id: &str, node_type: &str, args: Vec<FlowIrArg>) -> FlowIrStep {
            FlowIrStep::Node {
                id: id.to_string(),
                node_type: node_type.to_string(),
                args,
                continue_from: None,
                exec_arms: Vec::new(),
                anchor: None,
            }
        }

        fn param(name: &str) -> FlowIrParam {
            FlowIrParam {
                name: name.to_string(),
                value_type: acceptance_projection_type(),
            }
        }

        fn event(params: Vec<FlowIrParam>, steps: Vec<FlowIrStep>) -> FlowIrModule {
            FlowIrModule::Event {
                name: "eventsGeneric".to_string(),
                node_type: "events_generic".to_string(),
                params,
                steps,
                anchor: None,
            }
        }

        let reviewer = "example@example.com";
        let program = FlowIrProgram {
            modules: vec![
                FlowIrModule::Event {
                    name: "eventsSimple".to_string(),
                    node_type: "events_simple".to_string(),
                    params: Vec::new(),
                    steps: vec![node(
                        "initial_review",
                        "smtp_email_send",
                        vec![
                            arg("to", literal(reviewer)),
                            arg("ticket_id", reference("ticket_id")),
                        ],
                    )],
                    anchor: None,
                },
                event(
                    vec![param("payload"), param("ticketId"), param("draftReply")],
                    vec![node(
                        "send_customer",
                        "smtp_email_send",
                        vec![
                            arg("to", reference("requester_email")),
                            arg("body", reference("draftReply")),
                        ],
                    )],
                ),
                event(
                    vec![
                        param("payload"),
                        param("ticketId"),
                        param("revisionFeedback"),
                    ],
                    vec![
                        node(
                            "regenerate",
                            "ai_model_generate",
                            vec![arg("feedback", reference("revisionFeedback"))],
                        ),
                        node(
                            "reask_reviewer",
                            "smtp_email_send",
                            vec![
                                arg("to", literal(reviewer)),
                                arg("ticket_id", reference("ticketId")),
                            ],
                        ),
                    ],
                ),
            ],
            ..Default::default()
        };
        let catalog = vec![
            metadata("events_simple", Vec::new(), Vec::new()),
            metadata("events_generic", Vec::new(), Vec::new()),
            metadata("smtp_email_send", Vec::new(), Vec::new()),
            metadata("ai_model_generate", Vec::new(), Vec::new()),
        ];
        let catalog_by_name = catalog
            .iter()
            .map(|metadata| (normalize(&metadata.name), metadata))
            .collect::<HashMap<_, _>>();
        let contract = RequestApprovalLoopContract {
            reviewer_emails: vec![reviewer.to_string()],
            channel: RequestApprovalChannel::PageAction,
        };

        assert!(
            ui_approval_loop_diagnostics(&contract, &program, &catalog_by_name).is_empty(),
            "UI diagnostics: {:#?}",
            ui_approval_loop_diagnostics(&contract, &program, &catalog_by_name)
        );
    }

    #[test]
    fn mixed_imap_and_smtp_list_item_becomes_two_protocol_requirements() {
        let contract = derive_request_acceptance_contract(
            "1. Cron Job, Email abruf auf Email 1 (IMAP) - grundsätzlich IMAP und SMTP\n\
             2. Das Modell beantwortet die Supportanfrage",
        );
        assert!(contract.criteria.iter().any(|criterion| {
            criterion.actions == ["schedule"] && criterion.objects == ["schedule"]
        }));
        assert!(contract.criteria.iter().any(|criterion| {
            criterion.actions == ["read"]
                && criterion.objects.iter().any(|object| object == "imap")
                && !criterion.objects.iter().any(|object| object == "smtp")
        }));
        assert!(contract.criteria.iter().any(|criterion| {
            criterion.actions == ["send"]
                && criterion.objects.iter().any(|object| object == "smtp")
                && !criterion.objects.iter().any(|object| object == "imap")
        }));
    }

    #[test]
    fn complete_required_plan_passes_acceptance_and_can_start_a_draft() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let binding = store.bind_request_acceptance_contract(
            &board.id,
            "Format the customer message, then send a Slack notification.",
        );
        let response = store.begin_with_acceptance_binding(
            &board,
            &acceptance_catalog(),
            BeginFlowIrDraftArgs {
                draft_id: "complete-contract".to_string(),
                replace_existing: false,
                expected_modules: vec!["eventsSimple".to_string()],
                capability_plan: acceptance_capability_plan(true),
                mode: FlowIrDraftMode::Additive,
                program: acceptance_program(),
            },
            &binding,
        );
        assert_ne!(
            response.code.as_deref(),
            Some("IR_REQUEST_ACCEPTANCE_CONTRACT_INCOMPLETE")
        );
        assert_eq!(response.revision, Some(0));
    }

    #[test]
    fn model_plan_rewrite_cannot_rewrite_the_host_contract() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let binding = store.bind_request_acceptance_contract(
            &board.id,
            "Format the customer message, then send a Slack notification.",
        );
        let started = store.begin_with_acceptance_binding(
            &board,
            &acceptance_catalog(),
            BeginFlowIrDraftArgs {
                draft_id: "retained-contract".to_string(),
                replace_existing: false,
                expected_modules: vec!["eventsSimple".to_string()],
                capability_plan: acceptance_capability_plan(true),
                mode: FlowIrDraftMode::Additive,
                program: acceptance_program(),
            },
            &binding,
        );
        assert_eq!(started.revision, Some(0));
        let response = store.update_draft(
            &board,
            &acceptance_catalog(),
            UpdateFlowIrDraftArgs {
                draft_id: "retained-contract".to_string(),
                expected_revision: 0,
                expected_modules: None,
                capability_plan: Some(acceptance_capability_plan(false)),
                interfaces: Vec::new(),
                variables: Vec::new(),
                remove_modules: Vec::new(),
                remove_interfaces: Vec::new(),
                remove_variables: Vec::new(),
                allow_scope_reduction: true,
            },
        );
        assert_eq!(response.status, "draft_updated");
        assert_eq!(response.revision, Some(1));
        assert_eq!(
            store
                .drafts
                .lock()
                .unwrap()
                .get("retained-contract")
                .unwrap()
                .request_acceptance_contract
                .criteria
                .iter()
                .filter(|criterion| criterion.objects.contains(&"slack".to_string()))
                .count(),
            1
        );
    }

    #[test]
    fn rebinding_or_clearing_pending_contract_cannot_weaken_an_existing_draft() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let original = store.bind_request_acceptance_contract(
            &board.id,
            "Format the customer message, then send a Slack notification.",
        );
        let started = store.begin_with_acceptance_binding(
            &board,
            &acceptance_catalog(),
            BeginFlowIrDraftArgs {
                draft_id: "old-request".to_string(),
                replace_existing: false,
                expected_modules: vec!["eventsSimple".to_string()],
                capability_plan: acceptance_capability_plan(true),
                mode: FlowIrDraftMode::Additive,
                program: acceptance_program(),
            },
            &original,
        );
        assert_eq!(started.revision, Some(0));
        let unrelated = store.bind_request_acceptance_contract(
            &board.id,
            "Delete expired database records, then publish an audit report.",
        );
        let empty = store.bind_request_acceptance_contract(&board.id, "continue");
        assert_eq!(empty.criterion_count(), 0);
        assert!(store.release_request_acceptance_contract(&unrelated));
        assert!(store.release_request_acceptance_contract(&empty));

        let weakened = store.upsert_module(
            &board,
            &acceptance_catalog(),
            UpsertFlowIrModuleArgs {
                draft_id: "old-request".to_string(),
                expected_revision: 0,
                allow_scope_reduction: true,
                module: program("customer message").modules.remove(0),
            },
        );
        assert_eq!(weakened.revision, Some(1));
        let validated = store.validate(
            &board,
            &acceptance_catalog(),
            ValidateFlowIrDraftArgs {
                draft_id: "old-request".to_string(),
                include_header: false,
                modules: Vec::new(),
            },
        );
        assert!(
            validated.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "IR_REQUEST_ACCEPTANCE_CONTRACT_INCOMPLETE"
            })
        );

        let result = store.commit(
            &board,
            &acceptance_catalog(),
            CommitFlowIrDraftArgs {
                draft_id: "old-request".to_string(),
                expected_revision: 1,
                allow_deletions: false,
                remove_node_ids: Vec::new(),
                remove_variable_ids: Vec::new(),
                remove_layer_ids: Vec::new(),
                remove_comment_ids: Vec::new(),
                use_best_candidate: false,
            },
        );
        assert_eq!(result.status, "validation_errors");
        assert!(result.commands.is_empty());
    }

    #[test]
    fn pending_acceptance_bindings_stay_hard_bounded_when_all_are_claimed() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let first = store.bind_request_acceptance_contract(&board.id, "Send a Slack notification.");
        store
            .claim_request_acceptance_contract(&board.id, "draft_0", Some(&first))
            .unwrap();
        for index in 1..=MAX_FLOW_IR_ACCEPTANCE_CONTRACTS_PER_STORE {
            let binding =
                store.bind_request_acceptance_contract(&board.id, "Send a Slack notification.");
            store
                .claim_request_acceptance_contract(
                    &board.id,
                    &format!("draft_{index}"),
                    Some(&binding),
                )
                .unwrap();
        }
        assert_eq!(
            store.request_acceptance_contracts.lock().unwrap().len(),
            MAX_FLOW_IR_ACCEPTANCE_CONTRACTS_PER_STORE
        );
        assert_eq!(
            store
                .claim_request_acceptance_contract(&board.id, "draft_0", Some(&first))
                .unwrap_err()
                .0,
            "IR_ACCEPTANCE_BINDING_INVALID"
        );
    }

    #[test]
    fn concurrent_board_requests_capture_only_their_own_acceptance_contract() {
        let store = Arc::new(FlowIrDraftStore::new());
        let board = Arc::new(empty_board());
        let mut live_catalog = acceptance_catalog();
        live_catalog.push(metadata(
            "email_send",
            vec![pin("message", "String")],
            Vec::new(),
        ));
        let live_catalog = Arc::new(live_catalog);
        let slack_binding = store.bind_request_acceptance_contract(
            &board.id,
            "Format the customer message, then send a Slack notification.",
        );
        let email_binding = store.bind_request_acceptance_contract(
            &board.id,
            "Format the customer message, then send an email notification.",
        );
        let mut email_program = acceptance_program();
        let FlowIrModule::Event { steps, .. } = &mut email_program.modules[0] else {
            unreachable!()
        };
        let FlowIrStep::Node { node_type, .. } = &mut steps[1] else {
            unreachable!()
        };
        *node_type = "email_send".to_string();
        let mut email_plan = acceptance_capability_plan(false);
        email_plan.requirements.push(FlowCapabilityRequirement {
            id: "send_email_notification".to_string(),
            intent: "send an email notification".to_string(),
            required: true,
            exact_node_type: Some("email_send".to_string()),
            inputs: Vec::new(),
            outputs: Vec::new(),
        });

        std::thread::scope(|scope| {
            let email_store = store.clone();
            let email_board = board.clone();
            let email_catalog = live_catalog.clone();
            scope.spawn(move || {
                email_store.begin_with_acceptance_binding(
                    &email_board,
                    &email_catalog,
                    BeginFlowIrDraftArgs {
                        draft_id: "email-request".to_string(),
                        replace_existing: false,
                        expected_modules: vec!["eventsSimple".to_string()],
                        capability_plan: email_plan,
                        mode: FlowIrDraftMode::Additive,
                        program: email_program,
                    },
                    &email_binding,
                )
            });
            let slack_store = store.clone();
            let slack_board = board.clone();
            let slack_catalog = live_catalog.clone();
            scope.spawn(move || {
                slack_store.begin_with_acceptance_binding(
                    &slack_board,
                    &slack_catalog,
                    BeginFlowIrDraftArgs {
                        draft_id: "slack-request".to_string(),
                        replace_existing: false,
                        expected_modules: vec!["eventsSimple".to_string()],
                        capability_plan: acceptance_capability_plan(true),
                        mode: FlowIrDraftMode::Additive,
                        program: acceptance_program(),
                    },
                    &slack_binding,
                )
            });
        });

        let drafts = store.drafts.lock().unwrap();
        let subjects = |draft_id: &str| {
            drafts[draft_id]
                .request_acceptance_contract
                .criteria
                .iter()
                .flat_map(|criterion| criterion.objects.iter().cloned())
                .collect::<HashSet<_>>()
        };
        assert!(subjects("slack-request").contains("slack"));
        assert!(!subjects("slack-request").contains("email"));
        assert!(subjects("email-request").contains("email"));
        assert!(!subjects("email-request").contains("slack"));
    }

    #[test]
    fn german_and_english_negations_never_become_positive_requirements() {
        let contract = derive_request_acceptance_contract(
            "Format the payload.\n- Erzeuge keinen Cron-Katalogknoten.\n- Never send email.",
        );
        assert!(
            contract
                .criteria
                .iter()
                .all(|criterion| { !criterion.summary.contains("Cron") || criterion.forbidden })
        );
        let email = contract
            .criteria
            .iter()
            .find(|criterion| criterion.objects.contains(&"email".to_string()))
            .expect("email prohibition retained");
        assert!(email.forbidden);
    }

    #[test]
    fn negated_approval_conditions_remain_positive_branches() {
        let contract = derive_request_acceptance_contract(
            "- If not approved, send an email for review.\n\
             - Wenn nicht freigegeben, den Entwurf anpassen und eine Freigabemail senden.",
        );
        assert!(!contract.criteria.is_empty());
        assert!(
            contract
                .criteria
                .iter()
                .all(|criterion| !criterion.forbidden),
            "conditional criteria: {:#?}",
            contract.criteria
        );
        assert!(contract.criteria.iter().any(|criterion| {
            criterion.actions.iter().any(|action| action == "send")
                && criterion.objects.iter().any(|object| object == "email")
        }));
    }

    #[test]
    fn recipient_scoped_email_ban_is_not_overgeneralized_to_reviewer_smtp() {
        let contract = derive_request_acceptance_contract(
            "- Keine Mail wird automatisch an den Kunden geschickt.\n\
             - Eine Freigabemail an Christian senden.",
        );
        assert_eq!(contract.criteria.len(), 1, "{:#?}", contract.criteria);
        assert!(!contract.criteria[0].forbidden);
        assert_eq!(contract.criteria[0].actions, ["send"]);
        assert_eq!(contract.criteria[0].objects, ["email"]);
    }

    #[test]
    fn forbidden_email_send_does_not_forbid_imap_reads() {
        let contract = derive_request_acceptance_contract("Never send email.");
        assert_eq!(contract.criteria.len(), 1);
        assert!(contract.criteria[0].forbidden);
        let catalog = vec![
            metadata("events_simple", Vec::new(), Vec::new()),
            metadata("imap_email_read", Vec::new(), Vec::new()),
            metadata("smtp_email_send", Vec::new(), Vec::new()),
        ];
        let with_node = |node_type: &str| FlowIrProgram {
            modules: vec![FlowIrModule::Event {
                name: "eventsSimple".to_string(),
                node_type: "events_simple".to_string(),
                params: Vec::new(),
                steps: vec![FlowIrStep::Node {
                    id: "mail".to_string(),
                    node_type: node_type.to_string(),
                    args: Vec::new(),
                    continue_from: None,
                    exec_arms: Vec::new(),
                    anchor: None,
                }],
                anchor: None,
            }],
            ..Default::default()
        };
        assert!(
            acceptance_contract_diagnostics(&contract, &with_node("imap_email_read"), &catalog)
                .is_empty()
        );
        assert_eq!(
            acceptance_contract_diagnostics(&contract, &with_node("smtp_email_send"), &catalog)[0]
                .code,
            "IR_REQUEST_ACCEPTANCE_CONTRACT_FORBIDDEN"
        );

        let mixed_action_contract = RequestAcceptanceContract {
            criteria: vec![RequestAcceptanceCriterion {
                summary: "do not send email while creating a log".to_string(),
                actions: vec!["send".to_string(), "create".to_string()],
                objects: vec!["email".to_string()],
                forbidden: true,
            }],
            ..Default::default()
        };
        assert!(
            acceptance_contract_diagnostics(
                &mixed_action_contract,
                &with_node("imap_email_read"),
                &catalog,
            )
            .is_empty()
        );
    }

    #[test]
    fn single_salient_request_and_cron_prohibition_are_enforced() {
        let slack = derive_request_acceptance_contract("Send a Slack notification.");
        assert_eq!(slack.criteria.len(), 1);
        assert_eq!(slack.criteria[0].objects, ["slack"]);

        let cron = derive_request_acceptance_contract("Do not create a cron node.");
        assert_eq!(cron.criteria.len(), 1);
        assert!(cron.criteria[0].forbidden);
        assert_eq!(cron.criteria[0].objects, ["cron_catalog"]);
        let program = FlowIrProgram {
            modules: vec![FlowIrModule::Event {
                name: "eventsCron".to_string(),
                node_type: "events_cron".to_string(),
                params: Vec::new(),
                steps: Vec::new(),
                anchor: None,
            }],
            ..Default::default()
        };
        assert_eq!(
            acceptance_contract_diagnostics(
                &cron,
                &program,
                &[metadata("events_cron", Vec::new(), Vec::new())],
            )[0]
            .code,
            "IR_REQUEST_ACCEPTANCE_CONTRACT_FORBIDDEN"
        );
    }

    #[test]
    fn external_schedule_intent_uses_simple_event_without_a_cron_catalog_node() {
        let contract = derive_request_acceptance_contract(
            "- Create a Cron job that reads IMAP email.\n\
             - The Cron schedule is registered externally.\n\
             - Do not create a cron catalog node.",
        );
        let catalog = vec![
            metadata("events_simple", Vec::new(), Vec::new()),
            metadata("events_cron", Vec::new(), Vec::new()),
            metadata("imap_email_read", Vec::new(), Vec::new()),
        ];
        let program = |event_type: &str| FlowIrProgram {
            modules: vec![FlowIrModule::Event {
                name: "entry".to_string(),
                node_type: event_type.to_string(),
                params: Vec::new(),
                steps: vec![FlowIrStep::Node {
                    id: "read_mail".to_string(),
                    node_type: "imap_email_read".to_string(),
                    args: Vec::new(),
                    continue_from: None,
                    exec_arms: Vec::new(),
                    anchor: None,
                }],
                anchor: None,
            }],
            ..Default::default()
        };
        assert!(
            acceptance_contract_diagnostics(&contract, &program("events_simple"), &catalog)
                .is_empty(),
            "contract: {:#?}",
            contract.criteria
        );
        assert!(
            acceptance_contract_diagnostics(&contract, &program("events_cron"), &catalog)
                .iter()
                .any(|diagnostic| {
                    diagnostic.code == "IR_REQUEST_ACCEPTANCE_CONTRACT_FORBIDDEN"
                })
        );
    }

    #[test]
    fn compiler_lowered_branch_and_iterator_cover_reachable_approval_and_retry() {
        let contract = derive_request_acceptance_contract(
            "- Approve the current ticket.\n\
             - Retry each item.",
        );
        let helper = FlowIrModule::Function {
            name: "reviewLoop".to_string(),
            params: Vec::new(),
            returns: Vec::new(),
            steps: vec![
                FlowIrStep::If {
                    id: "decision".to_string(),
                    condition: FlowIrValue::Literal {
                        value: super::super::ir::FlowIrLiteral::Boolean(true),
                    },
                    then_steps: Vec::new(),
                    else_steps: Vec::new(),
                    anchor: None,
                },
                FlowIrStep::ForEach {
                    id: "retry_loop".to_string(),
                    array: FlowIrValue::List { items: Vec::new() },
                    item: "item".to_string(),
                    index: None,
                    parallel: false,
                    steps: Vec::new(),
                    anchor: None,
                },
            ],
            anchor: None,
        };
        let event = |steps| FlowIrModule::Event {
            name: "eventsSimple".to_string(),
            node_type: "events_simple".to_string(),
            params: Vec::new(),
            steps,
            anchor: None,
        };
        let catalog = [metadata("events_simple", Vec::new(), Vec::new())];
        let unreachable = FlowIrProgram {
            modules: vec![helper.clone(), event(Vec::new())],
            ..Default::default()
        };
        assert_eq!(
            acceptance_contract_diagnostics(&contract, &unreachable, &catalog).len(),
            2
        );
        let called = FlowIrProgram {
            modules: vec![
                helper,
                event(vec![FlowIrStep::CallFunction {
                    id: "review".to_string(),
                    function: "reviewLoop".to_string(),
                    args: Vec::new(),
                    anchor: None,
                }]),
            ],
            ..Default::default()
        };
        assert!(acceptance_contract_diagnostics(&contract, &called, &catalog).is_empty());
    }

    #[test]
    fn approval_and_retry_are_not_covered_by_generic_send_or_call_nodes() {
        assert!(!acceptance_action_covered(
            "approve",
            &HashSet::from(["send"])
        ));
        assert!(!acceptance_action_covered(
            "reject",
            &HashSet::from(["send"])
        ));
        assert!(!acceptance_action_covered(
            "retry",
            &HashSet::from(["call"])
        ));
    }

    #[test]
    fn stale_draft_revision_is_rejected() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let started = store.begin(
            &board,
            &catalog(),
            BeginFlowIrDraftArgs {
                draft_id: "support".to_string(),
                replace_existing: false,
                expected_modules: vec!["eventsSimple".to_string()],
                capability_plan: capability_plan(),
                mode: FlowIrDraftMode::Additive,
                program: program("hello"),
            },
        );
        assert_eq!(started.revision, Some(0));
        let response = store.upsert_module(
            &board,
            &catalog(),
            UpsertFlowIrModuleArgs {
                draft_id: "support".to_string(),
                expected_revision: 99,
                allow_scope_reduction: false,
                module: program("updated").modules.remove(0),
            },
        );
        assert_eq!(response.code.as_deref(), Some("IR_REVISION_CONFLICT"));
    }

    #[test]
    fn expected_module_kind_is_part_of_the_retained_contract() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let mut wrong_kind = program("hello");
        wrong_kind.modules = vec![FlowIrModule::Function {
            name: "eventsSimple".to_string(),
            params: Vec::new(),
            returns: Vec::new(),
            steps: vec![FlowIrStep::Node {
                id: "message".to_string(),
                node_type: "string_format".to_string(),
                args: vec![FlowIrArg {
                    pin: "format_string".to_string(),
                    occurrence: 0,
                    value: FlowIrValue::Literal {
                        value: FlowIrLiteral::String("hello".to_string()),
                    },
                }],
                continue_from: None,
                exec_arms: Vec::new(),
                anchor: None,
            }],
            anchor: None,
        }];
        let response = store.begin(
            &board,
            &catalog(),
            BeginFlowIrDraftArgs {
                draft_id: "wrong-kind".to_string(),
                replace_existing: false,
                expected_modules: vec!["eventsSimple".to_string()],
                capability_plan: capability_plan(),
                mode: FlowIrDraftMode::Additive,
                program: wrong_kind,
            },
        );
        assert!(
            response
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "IR_MODULE_KIND_MISMATCH")
        );
        assert!(response.missing_modules.is_empty());
    }

    #[test]
    fn commit_refuses_missing_expected_modules() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        store.begin(
            &board,
            &catalog(),
            BeginFlowIrDraftArgs {
                draft_id: "support".to_string(),
                replace_existing: false,
                expected_modules: vec!["classify".to_string()],
                capability_plan: capability_plan(),
                mode: FlowIrDraftMode::Additive,
                program: program("hello"),
            },
        );
        let result = store.commit(
            &board,
            &catalog(),
            CommitFlowIrDraftArgs {
                draft_id: "support".to_string(),
                expected_revision: 0,
                allow_deletions: false,
                remove_node_ids: Vec::new(),
                remove_variable_ids: Vec::new(),
                remove_layer_ids: Vec::new(),
                remove_comment_ids: Vec::new(),
                use_best_candidate: false,
            },
        );
        assert_eq!(result.code.as_deref(), Some("IR_REQUIRED_MODULES_MISSING"));
    }

    #[test]
    fn begin_refuses_infeasible_capability_plan_without_storing_a_draft() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let response = store.begin(
            &board,
            &catalog(),
            BeginFlowIrDraftArgs {
                draft_id: "unsupported".to_string(),
                replace_existing: false,
                expected_modules: vec!["eventsSimple".to_string()],
                capability_plan: FlowCapabilityPlanRequest {
                    requirements: vec![FlowCapabilityRequirement {
                        id: "smtp_custom_headers".to_string(),
                        intent: "SMTP reply headers".to_string(),
                        required: true,
                        exact_node_type: Some("missing_smtp_with_headers".to_string()),
                        inputs: Vec::new(),
                        outputs: Vec::new(),
                    }],
                    modules: vec![super::super::ir::FlowModuleEstimate {
                        name: "eventsSimple".to_string(),
                        kind: FlowModuleKind::Event,
                        estimated_nodes: 1,
                    }],
                },
                mode: FlowIrDraftMode::Additive,
                program: program("hello"),
            },
        );
        assert_eq!(response.status, "infeasible");
        assert_eq!(
            response.code.as_deref(),
            Some("IR_CAPABILITY_PLAN_INFEASIBLE")
        );
        let commit = store.commit(
            &board,
            &catalog(),
            CommitFlowIrDraftArgs {
                draft_id: "unsupported".to_string(),
                expected_revision: 0,
                allow_deletions: false,
                remove_node_ids: Vec::new(),
                remove_variable_ids: Vec::new(),
                remove_layer_ids: Vec::new(),
                remove_comment_ids: Vec::new(),
                use_best_candidate: false,
            },
        );
        assert_eq!(commit.code.as_deref(), Some("IR_DRAFT_MISSING"));
    }

    #[test]
    fn begin_preserves_an_existing_draft_unless_replacement_is_explicit() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let begin_args = |message: &str, replace_existing| BeginFlowIrDraftArgs {
            draft_id: "retained".to_string(),
            replace_existing,
            expected_modules: vec!["eventsSimple".to_string()],
            capability_plan: capability_plan(),
            mode: FlowIrDraftMode::Additive,
            program: program(message),
        };
        assert_eq!(
            store
                .begin(&board, &catalog(), begin_args("original", false))
                .status,
            "draft_started"
        );
        let duplicate = store.begin(&board, &catalog(), begin_args("replacement", false));
        assert_eq!(duplicate.code.as_deref(), Some("IR_DRAFT_ALREADY_EXISTS"));
        assert_eq!(duplicate.revision, Some(0));
        assert_eq!(
            store
                .begin(&board, &catalog(), begin_args("replacement", true))
                .status,
            "draft_started"
        );
    }

    #[test]
    fn begin_requires_explicit_module_coverage_and_capability_contract() {
        let missing_plan = serde_json::from_value::<BeginFlowIrDraftArgs>(json!({
            "draft_id": "missing-plan",
            "expected_modules": ["eventsSimple"]
        }));
        assert!(missing_plan.is_err());
        let missing_modules_field = serde_json::from_value::<BeginFlowIrDraftArgs>(json!({
            "draft_id": "missing-modules",
            "capability_plan": capability_plan()
        }));
        assert!(missing_modules_field.is_err());

        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let no_modules = store.begin(
            &board,
            &catalog(),
            BeginFlowIrDraftArgs {
                draft_id: "no-modules".to_string(),
                replace_existing: false,
                expected_modules: Vec::new(),
                capability_plan: capability_plan(),
                mode: FlowIrDraftMode::Additive,
                program: program("hello"),
            },
        );
        assert_eq!(
            no_modules.code.as_deref(),
            Some("IR_EXPECTED_MODULES_REQUIRED")
        );

        let no_requirements = store.begin(
            &board,
            &catalog(),
            BeginFlowIrDraftArgs {
                draft_id: "no-requirements".to_string(),
                replace_existing: false,
                expected_modules: vec!["eventsSimple".to_string()],
                capability_plan: FlowCapabilityPlanRequest {
                    requirements: Vec::new(),
                    modules: Vec::new(),
                },
                mode: FlowIrDraftMode::Additive,
                program: program("hello"),
            },
        );
        assert_eq!(
            no_requirements.code.as_deref(),
            Some("IR_CAPABILITY_PLAN_REQUIRED")
        );
    }

    #[test]
    fn incomplete_candidates_never_become_best_or_bypass_selected_coverage() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let started = store.begin(
            &board,
            &catalog(),
            BeginFlowIrDraftArgs {
                draft_id: "coverage".to_string(),
                replace_existing: false,
                expected_modules: vec!["eventsSimple".to_string(), "classify".to_string()],
                capability_plan: capability_plan(),
                mode: FlowIrDraftMode::Additive,
                program: program("hello"),
            },
        );
        assert_eq!(started.missing_modules, vec!["classify"]);
        assert!(
            store
                .drafts
                .lock()
                .unwrap()
                .get("coverage")
                .unwrap()
                .best
                .is_none()
        );

        let invalid_required_module = FlowIrModule::Event {
            name: "classify".to_string(),
            node_type: "missing_event".to_string(),
            params: Vec::new(),
            steps: Vec::new(),
            anchor: None,
        };
        let upserted = store.upsert_module(
            &board,
            &catalog(),
            UpsertFlowIrModuleArgs {
                draft_id: "coverage".to_string(),
                expected_revision: 0,
                allow_scope_reduction: false,
                module: invalid_required_module,
            },
        );
        assert_eq!(upserted.revision, Some(1));
        assert!(
            store
                .drafts
                .lock()
                .unwrap()
                .get("coverage")
                .unwrap()
                .best
                .is_none()
        );

        let committed = store.commit(
            &board,
            &catalog(),
            CommitFlowIrDraftArgs {
                draft_id: "coverage".to_string(),
                expected_revision: 1,
                allow_deletions: false,
                remove_node_ids: Vec::new(),
                remove_variable_ids: Vec::new(),
                remove_layer_ids: Vec::new(),
                remove_comment_ids: Vec::new(),
                use_best_candidate: true,
            },
        );
        assert_eq!(committed.code.as_deref(), Some("IR_DRAFT_INVALID"));
        assert!(committed.commands.is_empty());
    }

    #[test]
    fn supported_but_unused_capabilities_block_best_and_commit() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let mut live_catalog = catalog();
        live_catalog.push(metadata(
            "log_info",
            vec![pin("exec_in", "Execution"), pin("message", "String")],
            vec![pin("exec_out", "Execution")],
        ));
        let unused_program = FlowIrProgram {
            modules: vec![FlowIrModule::Event {
                name: "eventsSimple".to_string(),
                node_type: "events_simple".to_string(),
                params: Vec::new(),
                steps: vec![FlowIrStep::Node {
                    id: "log".to_string(),
                    node_type: "log_info".to_string(),
                    args: vec![FlowIrArg {
                        pin: "message".to_string(),
                        occurrence: 0,
                        value: FlowIrValue::Literal {
                            value: FlowIrLiteral::String("still editing".to_string()),
                        },
                    }],
                    continue_from: None,
                    exec_arms: Vec::new(),
                    anchor: None,
                }],
                anchor: None,
            }],
            ..Default::default()
        };
        let started = store.begin(
            &board,
            &live_catalog,
            BeginFlowIrDraftArgs {
                draft_id: "unused-capability".to_string(),
                replace_existing: false,
                expected_modules: vec!["eventsSimple".to_string()],
                capability_plan: capability_plan(),
                mode: FlowIrDraftMode::Additive,
                program: unused_program,
            },
        );
        assert!(
            !started
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "IR_REQUIRED_CAPABILITY_UNUSED")
        );
        assert_eq!(started.remaining_capabilities, vec!["format_message"]);
        assert!(started.message.contains("Continue implementing"));
        assert!(
            store
                .drafts
                .lock()
                .unwrap()
                .get("unused-capability")
                .unwrap()
                .best
                .is_none()
        );

        let committed = store.commit(
            &board,
            &live_catalog,
            CommitFlowIrDraftArgs {
                draft_id: "unused-capability".to_string(),
                expected_revision: 0,
                allow_deletions: false,
                remove_node_ids: Vec::new(),
                remove_variable_ids: Vec::new(),
                remove_layer_ids: Vec::new(),
                remove_comment_ids: Vec::new(),
                use_best_candidate: true,
            },
        );
        assert_eq!(committed.code.as_deref(), Some("IR_DRAFT_INVALID"));
        assert!(
            committed
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "IR_REQUIRED_CAPABILITY_UNUSED")
        );
    }

    #[test]
    fn commit_is_replay_safe_and_locks_the_committed_draft() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        store.begin(
            &board,
            &catalog(),
            BeginFlowIrDraftArgs {
                draft_id: "once".to_string(),
                replace_existing: false,
                expected_modules: vec!["eventsSimple".to_string()],
                capability_plan: capability_plan(),
                mode: FlowIrDraftMode::Additive,
                program: program("hello"),
            },
        );
        let args = CommitFlowIrDraftArgs {
            draft_id: "once".to_string(),
            expected_revision: 0,
            allow_deletions: false,
            remove_node_ids: Vec::new(),
            remove_variable_ids: Vec::new(),
            remove_layer_ids: Vec::new(),
            remove_comment_ids: Vec::new(),
            use_best_candidate: false,
        };
        let first = store.commit(&board, &catalog(), args.clone());
        assert_eq!(first.status, "queued");
        assert!(!first.commands.is_empty());

        let retry = store.commit(&board, &catalog(), args);
        assert_eq!(retry.status, "already_queued");
        assert_eq!(retry.code.as_deref(), Some("IR_DRAFT_ALREADY_COMMITTED"));
        assert!(retry.commands.is_empty());

        let upsert = store.upsert_module(
            &board,
            &catalog(),
            UpsertFlowIrModuleArgs {
                draft_id: "once".to_string(),
                expected_revision: 0,
                allow_scope_reduction: false,
                module: program("changed").modules.remove(0),
            },
        );
        assert_eq!(upsert.code.as_deref(), Some("IR_DRAFT_ALREADY_COMMITTED"));
    }

    #[test]
    fn commit_retains_the_exact_batch_and_refuses_an_unbounded_claim() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        store.begin(
            &board,
            &catalog(),
            BeginFlowIrDraftArgs {
                draft_id: "bounded".to_string(),
                replace_existing: false,
                expected_modules: vec!["eventsSimple".to_string()],
                capability_plan: capability_plan(),
                mode: FlowIrDraftMode::Additive,
                program: program("hello"),
            },
        );
        let args = CommitFlowIrDraftArgs {
            draft_id: "bounded".to_string(),
            expected_revision: 0,
            allow_deletions: false,
            remove_node_ids: Vec::new(),
            remove_variable_ids: Vec::new(),
            remove_layer_ids: Vec::new(),
            remove_comment_ids: Vec::new(),
            use_best_candidate: false,
        };

        let before_size = {
            let drafts = store.drafts.lock().unwrap();
            stored_draft_size(drafts.get("bounded").unwrap())
        };
        let queued = store.commit(&board, &catalog(), args.clone());
        assert_eq!(queued.status, "queued");
        let claim_id = queued.claim_id.clone().expect("pending claim id");
        let base_fingerprint = board_fingerprint(&board);
        assert_eq!(
            store.latest_pending_commit_token(&board.id),
            Some(FlowIrCommitToken {
                board_id: board.id.clone(),
                draft_id: "bounded".to_string(),
                revision: 0,
                base_fingerprint: base_fingerprint.clone(),
                claim_id: claim_id.clone(),
                requires_destructive_approval: false,
            })
        );
        let retained = store
            .pending_commands_if_current(&board, "bounded", 0, &base_fingerprint, &claim_id)
            .expect("exact pending commands are retained");
        assert_eq!(
            serde_json::to_value(&retained).unwrap(),
            serde_json::to_value(&queued.commands).unwrap()
        );
        assert!(
            store
                .pending_commands_if_current(
                    &board,
                    "bounded",
                    0,
                    &base_fingerprint,
                    "forged-claim",
                )
                .is_none()
        );
        let pending_size = {
            let drafts = store.drafts.lock().unwrap();
            stored_draft_size(drafts.get("bounded").unwrap())
        };
        assert!(pending_size > before_size);

        assert!(store.release_commit_if_matches("bounded", 0, &base_fingerprint, &claim_id,));
        assert!(
            store
                .pending_commands_if_current(&board, "bounded", 0, &base_fingerprint, &claim_id,)
                .is_none()
        );
        let released_size = {
            let drafts = store.drafts.lock().unwrap();
            stored_draft_size(drafts.get("bounded").unwrap())
        };
        assert_eq!(released_size, before_size);

        // Fill the remaining store budget so the draft itself still fits but retaining the exact
        // batch would cross the cap. Commit must reject this before creating any claim state.
        let desired_padding_size = MAX_FLOW_IR_DRAFT_STORE_BYTES
            .saturating_sub(pending_size)
            .saturating_add(1);
        let mut padding = {
            let drafts = store.drafts.lock().unwrap();
            drafts.get("bounded").unwrap().clone()
        };
        padding.base_fingerprint.clear();
        let padding_overhead = stored_draft_size(&padding);
        assert!(desired_padding_size >= padding_overhead);
        padding.base_fingerprint = "x".repeat(desired_padding_size - padding_overhead);
        let padding_size = stored_draft_size(&padding);
        assert_eq!(padding_size, desired_padding_size);
        assert!(padding_size.saturating_add(released_size) <= MAX_FLOW_IR_DRAFT_STORE_BYTES);
        assert!(padding_size.saturating_add(pending_size) > MAX_FLOW_IR_DRAFT_STORE_BYTES);
        store
            .drafts
            .lock()
            .unwrap()
            .insert("padding".to_string(), padding);

        let rejected = store.commit(&board, &catalog(), args);
        assert_eq!(
            rejected.code.as_deref(),
            Some("IR_DRAFT_STORE_SIZE_LIMIT_EXCEEDED")
        );
        assert!(rejected.claim_id.is_none());
        assert!(rejected.commands.is_empty());
        let drafts = store.drafts.lock().unwrap();
        let target = drafts.get("bounded").unwrap();
        assert!(target.committed_revision.is_none());
        assert!(target.pending_revision.is_none());
        assert!(target.pending_claim_id.is_none());
        assert!(target.pending_commands.is_none());
    }

    #[test]
    fn replacement_commit_requires_host_destructive_approval_even_without_removals() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        store.begin(
            &board,
            &catalog(),
            BeginFlowIrDraftArgs {
                draft_id: "replacement-review".to_string(),
                replace_existing: false,
                expected_modules: vec!["eventsSimple".to_string()],
                capability_plan: capability_plan(),
                mode: FlowIrDraftMode::Replace,
                program: program("hello"),
            },
        );
        let queued = store.commit(
            &board,
            &catalog(),
            CommitFlowIrDraftArgs {
                draft_id: "replacement-review".to_string(),
                expected_revision: 0,
                allow_deletions: false,
                remove_node_ids: Vec::new(),
                remove_variable_ids: Vec::new(),
                remove_layer_ids: Vec::new(),
                remove_comment_ids: Vec::new(),
                use_best_candidate: false,
            },
        );
        assert_eq!(queued.status, "queued", "{queued:#?}");
        let claim_id = queued.claim_id.expect("replacement claim id");
        let base_fingerprint = queued
            .base_fingerprint
            .expect("replacement base fingerprint");
        assert_eq!(
            store.pending_commit_requires_destructive_approval(
                "replacement-review",
                0,
                &base_fingerprint,
                &claim_id,
            ),
            Some(true),
        );
        assert!(
            store
                .latest_pending_commit_token(&board.id)
                .expect("replacement review token")
                .requires_destructive_approval
        );
    }

    #[test]
    fn host_can_release_an_unqueued_commit_claim_for_the_same_revision() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        store.begin(
            &board,
            &catalog(),
            BeginFlowIrDraftArgs {
                draft_id: "cancelled".to_string(),
                replace_existing: false,
                expected_modules: vec!["eventsSimple".to_string()],
                capability_plan: capability_plan(),
                mode: FlowIrDraftMode::Additive,
                program: program("hello"),
            },
        );
        let args = CommitFlowIrDraftArgs {
            draft_id: "cancelled".to_string(),
            expected_revision: 0,
            allow_deletions: false,
            remove_node_ids: Vec::new(),
            remove_variable_ids: Vec::new(),
            remove_layer_ids: Vec::new(),
            remove_comment_ids: Vec::new(),
            use_best_candidate: false,
        };
        let queued = store.commit(&board, &catalog(), args.clone());
        assert_eq!(queued.status, "queued");
        let claim_id = queued
            .claim_id
            .expect("queued commit carries a unique delivery generation");
        let base_fingerprint = board_fingerprint(&board);
        assert!(!store.release_commit_if_matches(
            "cancelled",
            0,
            &base_fingerprint,
            "forged-claim",
        ));
        assert!(store.release_commit_if_matches("cancelled", 0, &base_fingerprint, &claim_id,));
        let retried = store.commit(&board, &catalog(), args);
        assert_eq!(retried.status, "queued");
        let retried_claim_id = retried
            .claim_id
            .expect("a retried delivery receives a new generation");
        assert_ne!(retried_claim_id, claim_id);
        assert!(!store.release_commit_if_matches("cancelled", 0, &base_fingerprint, &claim_id,));
        assert!(
            store.pending_commit_matches("cancelled", 0, &base_fingerprint, &retried_claim_id,)
        );
    }

    #[test]
    fn pending_commit_requires_explicit_applied_ack_after_board_advances() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        store.begin(
            &board,
            &catalog(),
            BeginFlowIrDraftArgs {
                draft_id: "observed".to_string(),
                replace_existing: false,
                expected_modules: vec!["eventsSimple".to_string()],
                capability_plan: capability_plan(),
                mode: FlowIrDraftMode::Additive,
                program: program("hello"),
            },
        );
        let args = CommitFlowIrDraftArgs {
            draft_id: "observed".to_string(),
            expected_revision: 0,
            allow_deletions: false,
            remove_node_ids: Vec::new(),
            remove_variable_ids: Vec::new(),
            remove_layer_ids: Vec::new(),
            remove_comment_ids: Vec::new(),
            use_best_candidate: false,
        };
        let queued = store.commit(&board, &catalog(), args.clone());
        assert_eq!(queued.status, "queued");
        let claim_id = queued
            .claim_id
            .expect("queued commit carries a unique delivery generation");
        assert!(store.has_pending_commit());
        assert_eq!(store.observe_board(&board), 0);
        assert!(store.has_pending_commit());
        let base_fingerprint = board_fingerprint(&board);
        assert!(store.pending_commit_matches("observed", 0, &base_fingerprint, &claim_id));
        assert!(!store.pending_commit_matches("observed", 1, &base_fingerprint, &claim_id));
        assert!(!store.pending_commit_matches("observed", 0, "forged-base", &claim_id));
        assert!(!store.pending_commit_matches("observed", 0, &base_fingerprint, "forged-claim"));
        assert!(store.pending_commit_is_current(
            &board,
            "observed",
            0,
            &base_fingerprint,
            &claim_id,
        ));
        assert!(
            store
                .pending_commands_if_current(&board, "observed", 0, &base_fingerprint, &claim_id,)
                .is_some()
        );

        let mut advanced = board.clone();
        let mut variable = Variable::new("revisionMarker", VariableType::String, ValueType::Normal);
        variable.id = "revision-marker".to_string();
        advanced.variables.insert(variable.id.clone(), variable);
        assert_eq!(store.observe_board(&advanced), 1);
        assert!(store.has_pending_commit());
        assert!(!store.pending_commit_is_current(
            &advanced,
            "observed",
            0,
            &base_fingerprint,
            &claim_id,
        ));
        assert!(store.acknowledge_applied_commit(
            &advanced,
            "observed",
            0,
            &base_fingerprint,
            &claim_id,
        ));
        assert!(!store.has_pending_commit());
        assert!(!store.pending_commit_matches("observed", 0, &base_fingerprint, &claim_id));
        assert!(
            store
                .pending_commands_if_current(
                    &advanced,
                    "observed",
                    0,
                    &base_fingerprint,
                    &claim_id,
                )
                .is_none()
        );

        // Explicit Apply acknowledgement releases cache retention and board-wide exclusion, not
        // the exact-revision idempotency record.
        let retry = store.commit(&advanced, &catalog(), args);
        assert_eq!(retry.status, "already_queued");
        assert!(retry.commands.is_empty());
    }

    #[test]
    fn module_scope_reduction_requires_explicit_opt_in() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let mut larger_program = program("hello");
        let FlowIrModule::Event { steps, .. } = &mut larger_program.modules[0] else {
            unreachable!("test program is an event")
        };
        steps.push(FlowIrStep::Node {
            id: "second_message".to_string(),
            node_type: "string_format".to_string(),
            args: vec![FlowIrArg {
                pin: "format_string".to_string(),
                occurrence: 0,
                value: FlowIrValue::Literal {
                    value: FlowIrLiteral::String("world".to_string()),
                },
            }],
            continue_from: None,
            exec_arms: Vec::new(),
            anchor: None,
        });
        store.begin(
            &board,
            &catalog(),
            BeginFlowIrDraftArgs {
                draft_id: "scope".to_string(),
                replace_existing: false,
                expected_modules: vec!["eventsSimple".to_string()],
                capability_plan: capability_plan(),
                mode: FlowIrDraftMode::Additive,
                program: larger_program,
            },
        );
        let mut same_count_module = program("replacement").modules.remove(0);
        let FlowIrModule::Event { steps, .. } = &mut same_count_module else {
            unreachable!("test program is an event")
        };
        let FlowIrStep::Node { id, .. } = &mut steps[0] else {
            unreachable!("test program starts with a node")
        };
        *id = "replacement_message".to_string();
        steps.push(FlowIrStep::Node {
            id: "replacement_second".to_string(),
            node_type: "string_format".to_string(),
            args: vec![FlowIrArg {
                pin: "format_string".to_string(),
                occurrence: 0,
                value: FlowIrValue::Literal {
                    value: FlowIrLiteral::String("replacement".to_string()),
                },
            }],
            continue_from: None,
            exec_arms: Vec::new(),
            anchor: None,
        });
        let same_count_blocked = store.upsert_module(
            &board,
            &catalog(),
            UpsertFlowIrModuleArgs {
                draft_id: "scope".to_string(),
                expected_revision: 0,
                allow_scope_reduction: false,
                module: same_count_module,
            },
        );
        assert_eq!(
            same_count_blocked.code.as_deref(),
            Some("IR_SCOPE_REDUCTION_NOT_ALLOWED")
        );
        assert_eq!(same_count_blocked.revision, Some(0));

        let smaller_module = program("hello").modules.remove(0);
        let blocked = store.upsert_module(
            &board,
            &catalog(),
            UpsertFlowIrModuleArgs {
                draft_id: "scope".to_string(),
                expected_revision: 0,
                allow_scope_reduction: false,
                module: smaller_module.clone(),
            },
        );
        assert_eq!(
            blocked.code.as_deref(),
            Some("IR_SCOPE_REDUCTION_NOT_ALLOWED")
        );
        assert_eq!(blocked.revision, Some(0));

        let allowed = store.upsert_module(
            &board,
            &catalog(),
            UpsertFlowIrModuleArgs {
                draft_id: "scope".to_string(),
                expected_revision: 0,
                allow_scope_reduction: true,
                module: smaller_module,
            },
        );
        assert_eq!(allowed.revision, Some(1));
    }
}
