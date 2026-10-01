//! Workflow budgets, progress checkpoints, and retained loop state.

use super::workflow_diagnostics::{
    flowscript_repair_fingerprint, workflow_result_diagnostics,
    workflow_result_repair_declarations, workflow_result_structured_diagnostics,
};
use super::workflow_sdk::typed_ir_operation_budget;
use flow_like::flow::copilot::{
    BoardContextManifest, BoardScopePlan, FlowScriptCandidateRegression, FlowScriptRepairTracker,
    PlanBoardScopeArgs, ScopePlanRejection, ScopeStrategy, WorkflowArtifactKind, WorkflowSession,
    WorkflowSessionPolicy, WorkflowSessionSnapshot, accept_scope_plan,
    workflow_strategy_fingerprint,
};
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

/// Wall-clock budget for one nested delegated FlowPilot run.
/// It must stay well below the outer 30-minute bridge dispatch bound so budget exhaustion reaches
/// the waiting agent as a terminal, actionable incomplete result (retained draft coordinates plus
/// diagnostics) instead of an opaque outer-channel timeout after a burned turn.
pub(super) const NESTED_RUN_WALL_CLOCK_BUDGET: Duration = Duration::from_secs(12 * 60);

pub(super) const MAX_EXTERNAL_WORKFLOW_CONTINUATIONS: u8 = 2;

// Once usable declarations AND an accepted scope plan exist, a provider phase must dispatch its
// first source checkpoint within this soft bound. Planning and admitted ancillary reads do not
// consume it. Five minutes leaves enough room for a high-reasoning provider to compose and encode
// a substantial first tool call; a shorter bound repeatedly kills that call before dispatch and
// burns one of only two continuations. The shared 12-minute run ceiling remains authoritative.
pub(super) const EXTERNAL_PREDRAFT_SOURCE_CHECKPOINT_BUDGET: Duration = Duration::from_secs(5 * 60);

pub(super) const MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS: u8 = 12;

pub(super) const MAX_EXTERNAL_WORKFLOW_STALLED_EDIT_ATTEMPTS: u8 = 3;

// A continuation phase whose instructions demand more patching must actually be executable:
// exhausted stall/operation budgets receive this small bounded headroom instead of arriving dead.
pub(super) const EXTERNAL_CONTINUATION_OPERATION_HEADROOM: u16 = 6;

pub(super) const EXTERNAL_CONTINUATION_CHECK_HEADROOM: u8 = 2;

// Provider phases that failed transiently before the CLI issued a single tool call did no work;
// they are retried on their own bounded counter instead of consuming workflow continuations.
pub(super) const MAX_EXTERNAL_ZERO_ACTIVITY_RESTARTS: u8 = 2;

// Transport failures mid-phase (after real tool calls) get a slightly larger allowance: the MCP
// bridge retains all draft state, so a resumed phase loses nothing but the dropped stream.
pub(super) const MAX_EXTERNAL_TRANSPORT_RESTARTS: u8 = 3;

pub(super) const EXTERNAL_TRANSIENT_RESTART_BACKOFF: Duration = Duration::from_secs(2);

// Grace window between the shared circuit opening and the host ending the provider phase, so a
// CLI that honors "stop_for_host_continuation" can finish its turn cleanly first.
pub(super) const EXTERNAL_CIRCUIT_OPEN_PHASE_END_GRACE: Duration = Duration::from_secs(20);

// Count every model-dispatched source lifecycle operation, not only compiler checks. Otherwise a
// provider can alternate whole-document writes and patches forever without consuming the older
// validation-attempt budget.
pub(super) const MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS: u16 = 24;

pub(super) const MAX_EXTERNAL_FLOWSCRIPT_COMMIT_ATTEMPTS: u8 = 3;

// A segmented build pays the write/check cycle once per segment, so the flat single-draft budgets
// would starve it halfway through a plan it was told to make. Each additional segment earns a
// bounded slice; the ceilings keep a large plan from turning into an unbounded repair loop and keep
// the nested wall clock well under the outer 30-minute bridge dispatch bound.
pub(super) const EXTERNAL_SEGMENT_WALL_CLOCK_ALLOWANCE: Duration = Duration::from_secs(3 * 60);

pub(super) const MAX_EXTERNAL_SEGMENTED_WALL_CLOCK_BUDGET: Duration = Duration::from_secs(22 * 60);

pub(super) const EXTERNAL_SEGMENT_OPERATION_ALLOWANCE: u16 = 6;

pub(super) const MAX_EXTERNAL_SEGMENTED_FLOWSCRIPT_OPERATION_ATTEMPTS: u16 = 48;

pub(super) const EXTERNAL_SEGMENT_CHECK_ALLOWANCE: u8 = 3;

pub(super) const MAX_EXTERNAL_SEGMENTED_WORKFLOW_EDIT_ATTEMPTS: u8 = 24;

// A scope plan is one call, plus at most one bounded re-plan of the work that has not reached the
// board yet. Anything beyond that is the model planning instead of building.
pub(super) const MAX_EXTERNAL_SCOPE_PLAN_CALLS: u8 = 2;

// A malformed proposal must stay fixable, so rejections do not consume the revision budget above.
// They get their own small bound instead, so a model that cannot produce a valid plan stops
// reshaping it and reports what is blocking one.
pub(super) const MAX_EXTERNAL_SCOPE_PLAN_REJECTIONS: u8 = 4;

// Fraction of the nested wall clock after which a staged plan stops growing its draft and commits
// the coherent prefix it already validated, so a long build degrades to real partial progress
// instead of losing every segment at the deadline.
pub(super) const EXTERNAL_STAGED_COMMIT_PREFIX_RATIO: f64 = 0.7;

// Some applications genuinely take hours to build. Time is therefore EARNED rather than granted up
// front: a run that keeps demonstrating forward movement ratchets its deadline one slice at a time,
// while a run that circles produces the same progress mark and is cut off at its current deadline.
// None of the progress-based circuit breakers (repeated compiler states, the zero-progress circuit,
// the repeat-exhausted-budget rule) are relaxed by an extension — only volume budgets are.
pub(super) const EXTERNAL_TIME_EXTENSION_SLICE: Duration = Duration::from_secs(30 * 60);

pub(super) const MAX_EXTERNAL_EARNED_WALL_CLOCK: Duration = Duration::from_secs(8 * 60 * 60);

// A longer run legitimately needs more write/check/commit volume, so each earned slice raises those
// ceilings too. Without this the count budgets end a productive run inside the first hour no matter
// how much wall clock it has.
pub(super) const EXTERNAL_EXTENSION_OPERATION_GRANT: u16 = 6;

pub(super) const EXTERNAL_EXTENSION_CHECK_GRANT: u8 = 3;

pub(super) const EXTERNAL_EXTENSION_COMMIT_GRANT: u8 = 1;

pub(super) const EXTERNAL_EXTENSION_CONTINUATION_GRANT: u8 = 1;

// A big board pays more operations/edits for the same amount of behavior change, so the volume
// budgets scale with node count (one op per 8 nodes, one edit per 16), bounded so size never buys
// unbounded circling headroom. Stall/circuit cut-offs are unaffected.
const EXTERNAL_BOARD_SIZE_OPERATION_ALLOWANCE_CAP: u16 = 12;

const EXTERNAL_BOARD_SIZE_EDIT_ALLOWANCE_CAP: u8 = 6;

pub(super) const MAX_RETAINED_STRUCTURED_DIAGNOSTICS: usize = 12;

pub(super) const MAX_RETAINED_STRUCTURED_DIAGNOSTIC_BYTES: usize = 12_000;

// A typed build gets fixed lifecycle overhead plus roughly three operations per declared module
// (initial upsert and two repairs), while the fixed ceiling prevents an unbounded repair loop.
pub(super) const MIN_EXTERNAL_TYPED_IR_OPERATION_BUDGET: u16 = 24;

pub(super) const MAX_EXTERNAL_TYPED_IR_OPERATION_BUDGET: u16 = 64;

pub(super) const MAX_EXTERNAL_TYPED_IR_STALLED_ATTEMPTS: u8 = 3;

pub(super) const MAX_EXTERNAL_WORKFLOW_DECLARATION_CALLS: u8 =
    MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS.saturating_add(1);

pub(super) const MAX_INITIAL_DECLARATION_ATTEMPTS: u8 = 3;

pub(super) const MAX_EXTERNAL_PREDRAFT_CONTEXT_READS: u8 = 6;

pub(super) const MAX_REPAIR_DECLARATION_QUERIES: usize = 12;

pub(super) const MAX_REPAIR_DECLARATION_QUERY_BYTES: usize = 200;

pub(super) const MAX_REPAIR_DECLARATION_ATTEMPTS_PER_KEY: u8 = 2;

pub(super) const MAX_INJECTED_REPAIR_DECLARATIONS: usize = 32;

pub(super) const MAX_INJECTED_REPAIR_DECLARATION_BYTES: usize = 30_000;

pub(super) const MAX_RETAINED_DECLARATION_BYTES: usize = 48_000;

/// Comparable snapshot of forward movement in a board run.
///
/// This is the entire basis for earning more wall clock: an extension is granted only when the mark
/// strictly advanced since the previous grant. A run repairing the same diagnostics, rewriting the
/// same document, or re-reading the same context produces an identical mark and therefore buys no
/// more time. Every field only ever moves up, so "advanced" is unambiguous and cannot be gamed by
/// discarding work.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct WorkflowProgressMark {
    /// Planned segments whose commands reached the board.
    pub(super) committed_segments: usize,
    /// Planned segments authored into the retained draft.
    pub(super) authored_segments: usize,
    /// Revisions that checked clean. The strongest single progress signal there is.
    pub(super) valid_checks: u32,
    /// Size of the retained document. Grows as the build grows.
    pub(super) retained_source_len: usize,
    /// Distinct compiler states seen. Rises when repairs reach NEW diagnostics rather than
    /// revisiting old ones, which is exactly what circling fails to do.
    pub(super) distinct_repair_states: usize,
}

impl WorkflowProgressMark {
    pub(super) fn advanced_beyond(&self, previous: &Self) -> bool {
        self.committed_segments > previous.committed_segments
            || self.authored_segments > previous.authored_segments
            || self.valid_checks > previous.valid_checks
            || self.retained_source_len > previous.retained_source_len
            || self.distinct_repair_states > previous.distinct_repair_states
    }
}

/// Outcome of asking for more wall clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum TimeExtensionDecision {
    Granted {
        earned: Duration,
        grants: u8,
    },
    /// Nothing measurable moved since the last grant. This is the circling cut-off.
    NoProgress {
        earned: Duration,
    },
    /// The run is at the absolute ceiling; no amount of progress extends it further.
    CeilingReached {
        earned: Duration,
    },
    /// Already queued, or otherwise nothing left to spend time on.
    NotExtendable,
}

/// Segments a run was planned into. One when nothing was planned, so every budget below collapses
/// to its historical single-draft value on the unsegmented path.
fn planned_segment_count(plan: Option<&BoardScopePlan>) -> usize {
    plan.map_or(1, BoardScopePlan::segment_count)
}

fn extra_planned_segments(plan: Option<&BoardScopePlan>) -> u16 {
    u16::try_from(planned_segment_count(plan).saturating_sub(1)).unwrap_or(u16::MAX)
}

pub(super) fn scoped_operation_budget(plan: Option<&BoardScopePlan>) -> u16 {
    MAX_EXTERNAL_FLOWSCRIPT_OPERATION_ATTEMPTS
        .saturating_add(
            extra_planned_segments(plan).saturating_mul(EXTERNAL_SEGMENT_OPERATION_ALLOWANCE),
        )
        .min(MAX_EXTERNAL_SEGMENTED_FLOWSCRIPT_OPERATION_ATTEMPTS)
}

pub(super) fn scoped_edit_budget(plan: Option<&BoardScopePlan>) -> u8 {
    let extra = u8::try_from(extra_planned_segments(plan)).unwrap_or(u8::MAX);
    MAX_EXTERNAL_WORKFLOW_EDIT_ATTEMPTS
        .saturating_add(extra.saturating_mul(EXTERNAL_SEGMENT_CHECK_ALLOWANCE))
        .min(MAX_EXTERNAL_SEGMENTED_WORKFLOW_EDIT_ATTEMPTS)
}

/// A per-segment commit strategy needs one commit per segment plus the shared retry headroom.
pub(super) fn scoped_commit_budget(plan: Option<&BoardScopePlan>) -> u8 {
    let Some(plan) = plan else {
        return MAX_EXTERNAL_FLOWSCRIPT_COMMIT_ATTEMPTS;
    };
    if !plan.strategy.commits_per_segment() {
        return MAX_EXTERNAL_FLOWSCRIPT_COMMIT_ATTEMPTS;
    }
    let segments = u8::try_from(plan.segment_count()).unwrap_or(u8::MAX);
    MAX_EXTERNAL_FLOWSCRIPT_COMMIT_ATTEMPTS.saturating_add(segments)
}

/// Extra wall clock a plan's segments earn beyond the flat nested budget.
fn scoped_wall_clock_extension(plan: Option<&BoardScopePlan>) -> Duration {
    let extra = u32::from(extra_planned_segments(plan));
    let extension = EXTERNAL_SEGMENT_WALL_CLOCK_ALLOWANCE.saturating_mul(extra);
    let ceiling =
        MAX_EXTERNAL_SEGMENTED_WALL_CLOCK_BUDGET.saturating_sub(NESTED_RUN_WALL_CLOCK_BUDGET);
    extension.min(ceiling)
}

pub(super) fn submitted_flowscript(args: &serde_json::Value) -> Option<&str> {
    args.get("flowscript")
        .or_else(|| args.get("script"))
        .or_else(|| args.get("source"))
        .or_else(|| args.get("content"))
        .and_then(serde_json::Value::as_str)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WorkflowMutationPath {
    TypedIr,
    FlowScript,
    DirectCommands,
}

#[derive(Debug, Clone, Default)]
pub(super) struct WorkflowToolLoopSnapshot {
    pub(super) queued: bool,
    pub(super) last_flowscript: Option<String>,
    pub(super) last_declarations: Option<String>,
    pub(super) declaration_lookup_complete: bool,
    pub(super) unresolved_declaration_queries: Vec<String>,
    /// Exact live-catalog signatures injected by the latest FlowScript validation result. These
    /// are kept separately from the flattened diagnostic messages so a subprocess continuation
    /// does not lose the machine-actionable repair context.
    pub(super) last_repair_declarations: Vec<String>,
    pub(super) last_status: Option<String>,
    pub(super) last_errors: Vec<String>,
    pub(super) edit_attempts: u8,
    pub(super) flowscript_operation_attempts: u16,
    pub(super) stalled_edit_attempts: u8,
    pub(super) flowscript_commit_attempts: u8,
    pub(super) continuation_budget: Option<u8>,
    pub(super) edit_attempt_budget: Option<u8>,
    pub(super) flowscript_operation_budget: Option<u16>,
    pub(super) commit_attempt_budget: Option<u8>,
    pub(super) nested_wall_clock_budget: Option<Duration>,
    /// Human-readable name of the loop budget that is currently exhausted, if any. A continuation
    /// phase that starts in this state would be refused-on-arrival unless the host grants a fresh
    /// bounded slice first.
    pub(super) exhausted_budget: Option<String>,
    pub(super) last_structured_diagnostics: Vec<serde_json::Value>,
    pub(super) last_review_notes: usize,
    pub(super) modular_fallback: Option<FlowScriptCandidateRegression>,
    pub(super) retained_full_source: Option<String>,
    pub(super) flowscript_draft_id: Option<String>,
    pub(super) flowscript_draft_retained: bool,
    pub(super) flowscript_revision: Option<u64>,
    pub(super) typed_draft_id: Option<String>,
    pub(super) typed_draft_retained: bool,
    pub(super) typed_revision: Option<u64>,
    pub(super) typed_operation_attempts: u16,
    pub(super) typed_operation_budget: u16,
    #[allow(dead_code)] // carried across the phase handoff; no consumer restores them yet
    pub(super) typed_stalled_attempts: u8,
    pub(super) typed_missing_modules: Vec<String>,
    pub(super) mutation_path: Option<WorkflowMutationPath>,
    pub(super) shared_session: Option<WorkflowSessionSnapshot>,
    /// Accepted segmentation for this build. Absent until the model plans, and absent forever on a
    /// legacy single-shot path that never calls `plan_board_scope`.
    pub(super) scope_plan: Option<BoardScopePlan>,
    /// Set when a staged plan was told to commit the prefix it already validated because the wall
    /// clock is running out. The bridge continues the remaining segments in a fresh run.
    #[allow(dead_code)]
    pub(super) staged_prefix_commit_requested: bool,
    /// Wall-clock slices this run earned by demonstrating progress, and the total they bought.
    pub(super) granted_time_extensions: u8,
    pub(super) earned_wall_clock: Duration,
    pub(super) last_extension_rationale: Option<String>,
}

#[derive(Debug, Default)]
pub(super) struct WorkflowToolLoopState {
    pub(super) shared_session_started_at: Option<Instant>,
    pub(super) shared_session: Option<WorkflowSession>,
    pub(super) current_reads: u8,
    pub(super) predraft_context_reads: u8,
    pub(super) declaration_calls: u8,
    pub(super) declarations_since_edit: u8,
    pub(super) declaration_lookup_in_flight: bool,
    pub(super) initial_declaration_attempts: u8,
    pub(super) initial_declaration_lookup_usable: bool,
    pub(super) initial_declaration_lookup_complete: bool,
    pub(super) unresolved_declaration_queries: Vec<String>,
    pub(super) completed_repair_lookup_keys: HashSet<String>,
    pub(super) in_flight_repair_lookup_keys: HashSet<String>,
    pub(super) repair_lookup_attempts: HashMap<String, u8>,
    pub(super) edit_attempts: u8,
    pub(super) flowscript_operation_attempts: u16,
    pub(super) flowscript_commit_attempts: u8,
    pub(super) stalled_edit_attempts: u8,
    pub(super) has_previous_validation_result: bool,
    pub(super) previous_validation_diagnostics: HashSet<String>,
    pub(super) flowscript_seen_repair_signatures: HashSet<String>,
    pub(super) edit_in_flight: bool,
    /// Captured before invoking the frontend validator so a process/transport phase boundary cannot
    /// erase the only copy of a just-submitted rich draft.
    pub(super) in_flight_flowscript: Option<String>,
    pub(super) queued: bool,
    pub(super) last_flowscript: Option<String>,
    pub(super) repair_tracker: FlowScriptRepairTracker,
    pub(super) best_failed_errors: Vec<String>,
    pub(super) candidate_regression_warning: Option<String>,
    pub(super) pending_modular_fallback: Option<FlowScriptCandidateRegression>,
    pub(super) last_declarations: Option<String>,
    pub(super) last_repair_declarations: Vec<String>,
    pub(super) last_status: Option<String>,
    pub(super) last_errors: Vec<String>,
    pub(super) last_structured_diagnostics: Vec<serde_json::Value>,
    pub(super) last_review_notes: usize,
    pub(super) flowscript_draft_id: Option<String>,
    pub(super) flowscript_draft_retained: bool,
    pub(super) flowscript_revision: Option<u64>,
    pub(super) typed_draft_id: Option<String>,
    pub(super) typed_draft_retained: bool,
    pub(super) typed_revision: Option<u64>,
    pub(super) typed_operation_attempts: u16,
    pub(super) typed_expected_modules: usize,
    pub(super) typed_stalled_attempts: u8,
    pub(super) typed_missing_modules: Vec<String>,
    pub(super) typed_seen_repair_signatures: HashMap<String, HashSet<String>>,
    pub(super) mutation_path: Option<WorkflowMutationPath>,
    pub(super) scope_plan: Option<BoardScopePlan>,
    pub(super) scope_plan_calls: u8,
    pub(super) scope_plan_rejections: u8,
    pub(super) staged_prefix_commit_requested: bool,
    /// Wall-clock slices this run has earned by demonstrating progress.
    pub(super) granted_time_extensions: u8,
    /// Progress mark at the last grant. The next grant must strictly advance beyond it.
    pub(super) last_extension_progress: Option<WorkflowProgressMark>,
    /// Revisions that checked clean, tracked here because the progress ledger needs a monotone
    /// counter and `last_status` only holds the most recent value.
    pub(super) valid_checks: u32,
    /// The model's own account of what advanced and what remains, from its last extension request.
    /// Recorded for the user and telemetry; it never influences whether a grant is made.
    pub(super) last_extension_rationale: Option<String>,
    /// Node count of the board this run edits; sizes the operation/edit budgets so large boards
    /// earn more headroom up front instead of failing by exhaustion.
    pub(super) board_node_count: usize,
    /// Progress mark at the last SDK idle-continuation slice; the next repeat of the same
    /// exhausted budget is terminal only when progress did not advance beyond this mark.
    pub(super) last_idle_continuation_progress: Option<WorkflowProgressMark>,
}

impl WorkflowToolLoopState {
    pub(super) fn from_flowscript_recovery(
        recovery: Option<&flow_like::flow::copilot::FlowScriptDraftRecovery>,
    ) -> Self {
        let mut state = Self::default();
        let Some(recovery) = recovery else {
            return state;
        };
        if !matches!(
            recovery.status,
            flow_like::flow::copilot::FlowIrDraftRecoveryStatus::ExactMatch
        ) || !recovery.auto_resume
        {
            return state;
        }
        let Some(context) = recovery
            .exact_match
            .as_ref()
            .filter(|context| !context.stale_board)
        else {
            return state;
        };
        let Some(source) = context.source.as_deref() else {
            return state;
        };

        let status = if context.checked {
            "valid"
        } else {
            context.status.as_str()
        };
        let payload = serde_json::json!({
            "status": status,
            "diagnostics": &context.diagnostics,
        });
        let diagnostics = workflow_result_diagnostics(Some(&payload));
        state.last_structured_diagnostics = workflow_result_structured_diagnostics(Some(&payload));
        state.last_repair_declarations = workflow_result_repair_declarations(Some(&payload));
        state.last_status = Some(status.to_string());
        state.last_errors = diagnostics.clone();
        state.last_flowscript = Some(source.to_string());
        state.flowscript_draft_id = Some(context.draft_id.clone());
        state.flowscript_draft_retained = true;
        state.flowscript_revision = Some(context.revision);
        state.initial_declaration_lookup_usable = true;
        state.initial_declaration_lookup_complete = true;
        state.mutation_path = Some(WorkflowMutationPath::FlowScript);
        if !context.checked && !diagnostics.is_empty() {
            state
                .repair_tracker
                .record_failed_with_diagnostics(source, Some(diagnostics.len()));
            state.best_failed_errors = diagnostics.clone();
            state
                .flowscript_seen_repair_signatures
                .insert(flowscript_repair_fingerprint(
                    Some(status),
                    &diagnostics,
                    &state.last_structured_diagnostics,
                ));
        }
        state
    }

    pub(super) fn attach_shared_session(&mut self, manifest: Option<BoardContextManifest>) {
        let Some(manifest) = manifest else {
            return;
        };
        self.board_node_count = manifest.board.graph.nodes.len();
        let mut session = WorkflowSession::new(manifest, WorkflowSessionPolicy::default());
        let _ = session.mark_manifest_ready(0);
        let _ = session.begin_discovery(0);
        if self.flowscript_draft_retained
            && let (Some(draft_id), Some(revision), Some(source)) = (
                self.flowscript_draft_id.as_deref(),
                self.flowscript_revision,
                self.last_flowscript.as_deref(),
            )
        {
            let _ = session.record_artifact(
                WorkflowArtifactKind::FlowScript,
                draft_id,
                revision,
                workflow_strategy_fingerprint(&serde_json::json!({ "source": source })),
                0,
            );
            session.observe_flowscript_source(draft_id, revision, source);
        }
        self.shared_session_started_at = Some(Instant::now());
        self.shared_session = Some(session);
    }

    pub(super) fn shared_session_elapsed_ms(&self) -> u64 {
        self.shared_session_started_at
            .map(|started| started.elapsed().as_millis() as u64)
            .unwrap_or_default()
    }

    pub(super) fn needs_initial_declaration_coverage(&self) -> bool {
        // This is an explicit host-owned authorization bit. The first usable live-catalog result
        // unlocks a retained full-shape draft; complete coverage remains separate reporting data.
        // Compiler diagnostics, rather than exhaustive pre-draft discovery, drive later focused
        // lookups. Exact retained recovery seeds both bits in `from_flowscript_recovery`.
        !(self.initial_declaration_lookup_usable || self.initial_declaration_lookup_complete)
    }

    pub(super) fn snapshot(&self) -> WorkflowToolLoopSnapshot {
        let (retained_flowscript, retained_status, retained_errors) = if self.queued {
            (
                self.last_flowscript.clone(),
                Some("queued".to_string()),
                self.last_errors.clone(),
            )
        } else if self.flowscript_draft_retained && self.last_flowscript.is_some() {
            // The retained source store is authoritative for the code-first lifecycle. A richer
            // earlier failed candidate is useful for regression checks, but must not replace a
            // newer exact revision (especially a `valid` one) in continuation/recovery context.
            (
                self.last_flowscript.clone(),
                self.last_status.clone(),
                self.last_errors.clone(),
            )
        } else if let Some(best_failed) = self.repair_tracker.best_failed_source() {
            let mut errors = self.best_failed_errors.clone();
            if let Some(warning) = &self.candidate_regression_warning
                && !errors.contains(warning)
            {
                errors.push(warning.clone());
            }
            (
                Some(best_failed.to_string()),
                Some("validation_errors".to_string()),
                errors,
            )
        } else if let Some(in_flight) = self.in_flight_flowscript.as_deref() {
            (
                Some(in_flight.to_string()),
                Some("edit_interrupted".to_string()),
                vec![
                    "The provider or transport ended while the validator was running; resubmit this complete draft and repair any returned diagnostics."
                        .to_string(),
                ],
            )
        } else {
            (
                self.last_flowscript.clone(),
                self.last_status.clone(),
                self.last_errors.clone(),
            )
        };
        WorkflowToolLoopSnapshot {
            queued: self.queued,
            last_flowscript: retained_flowscript,
            last_declarations: self.last_declarations.clone(),
            declaration_lookup_complete: self.initial_declaration_lookup_complete,
            unresolved_declaration_queries: self.unresolved_declaration_queries.clone(),
            last_repair_declarations: self.last_repair_declarations.clone(),
            last_status: retained_status,
            last_errors: retained_errors,
            edit_attempts: self.edit_attempts,
            flowscript_operation_attempts: self.flowscript_operation_attempts,
            stalled_edit_attempts: self.stalled_edit_attempts,
            flowscript_commit_attempts: self.flowscript_commit_attempts,
            continuation_budget: Some(self.continuation_budget()),
            edit_attempt_budget: Some(self.edit_attempt_budget()),
            flowscript_operation_budget: Some(self.flowscript_operation_budget()),
            commit_attempt_budget: Some(self.commit_attempt_budget()),
            nested_wall_clock_budget: Some(
                NESTED_RUN_WALL_CLOCK_BUDGET.saturating_add(self.wall_clock_extension()),
            ),
            exhausted_budget: self.exhausted_budget(),
            last_structured_diagnostics: self.last_structured_diagnostics.clone(),
            last_review_notes: self.last_review_notes,
            modular_fallback: self
                .queued
                .then(|| self.pending_modular_fallback.clone())
                .flatten(),
            retained_full_source: self
                .flowscript_draft_retained
                .then_some(self.last_flowscript.as_deref())
                .flatten()
                .or_else(|| self.repair_tracker.best_failed_source())
                .or(self.in_flight_flowscript.as_deref())
                .map(str::to_string),
            flowscript_draft_id: self.flowscript_draft_id.clone(),
            flowscript_draft_retained: self.flowscript_draft_retained,
            flowscript_revision: self.flowscript_revision,
            typed_draft_id: self.typed_draft_id.clone(),
            typed_draft_retained: self.typed_draft_retained,
            typed_revision: self.typed_revision,
            typed_operation_attempts: self.typed_operation_attempts,
            typed_operation_budget: typed_ir_operation_budget(self.typed_expected_modules),
            typed_stalled_attempts: self.typed_stalled_attempts,
            typed_missing_modules: self.typed_missing_modules.clone(),
            mutation_path: self.mutation_path,
            shared_session: self
                .shared_session
                .as_ref()
                .map(|session| session.snapshot(self.shared_session_elapsed_ms())),
            scope_plan: self.scope_plan.clone(),
            staged_prefix_commit_requested: self.staged_prefix_commit_requested,
            granted_time_extensions: self.granted_time_extensions,
            earned_wall_clock: self.earned_wall_clock(),
            last_extension_rationale: self.last_extension_rationale.clone(),
        }
    }

    /// Extra source operations a large board earns: one per 8 nodes, bounded so a huge board
    /// cannot buy unbounded circling headroom.
    pub(super) fn board_size_operation_allowance(&self) -> u16 {
        u16::try_from(self.board_node_count / 8)
            .unwrap_or(u16::MAX)
            .min(EXTERNAL_BOARD_SIZE_OPERATION_ALLOWANCE_CAP)
    }

    /// Extra edit attempts a large board earns: one per 16 nodes, bounded.
    pub(super) fn board_size_edit_allowance(&self) -> u8 {
        u8::try_from(self.board_node_count / 16)
            .unwrap_or(u8::MAX)
            .min(EXTERNAL_BOARD_SIZE_EDIT_ALLOWANCE_CAP)
    }

    pub(super) fn flowscript_operation_budget(&self) -> u16 {
        scoped_operation_budget(self.scope_plan.as_ref())
            .saturating_add(
                u16::from(self.granted_time_extensions)
                    .saturating_mul(EXTERNAL_EXTENSION_OPERATION_GRANT),
            )
            .saturating_add(self.board_size_operation_allowance())
    }

    pub(super) fn edit_attempt_budget(&self) -> u8 {
        scoped_edit_budget(self.scope_plan.as_ref())
            .saturating_add(
                self.granted_time_extensions
                    .saturating_mul(EXTERNAL_EXTENSION_CHECK_GRANT),
            )
            .saturating_add(self.board_size_edit_allowance())
    }

    pub(super) fn commit_attempt_budget(&self) -> u8 {
        scoped_commit_budget(self.scope_plan.as_ref()).saturating_add(
            self.granted_time_extensions
                .saturating_mul(EXTERNAL_EXTENSION_COMMIT_GRANT),
        )
    }

    /// Provider continuations a long run may spend. Phases end for many reasons over hours, and a
    /// cap sized for a 12-minute run would strand a productive build. The rule that a continuation
    /// arriving on the SAME exhausted budget is terminal still applies, so this cannot mask circling.
    pub(super) fn continuation_budget(&self) -> u8 {
        MAX_EXTERNAL_WORKFLOW_CONTINUATIONS.saturating_add(
            self.granted_time_extensions
                .saturating_mul(EXTERNAL_EXTENSION_CONTINUATION_GRANT),
        )
    }

    /// Total wall clock beyond the flat nested budget: what the plan's segments earned up front,
    /// plus every slice progress has bought since.
    pub(super) fn wall_clock_extension(&self) -> Duration {
        let planned = scoped_wall_clock_extension(self.scope_plan.as_ref());
        let earned = self.earned_wall_clock();
        let ceiling = MAX_EXTERNAL_EARNED_WALL_CLOCK.saturating_sub(NESTED_RUN_WALL_CLOCK_BUDGET);
        planned.saturating_add(earned).min(ceiling)
    }

    pub(super) fn earned_wall_clock(&self) -> Duration {
        EXTERNAL_TIME_EXTENSION_SLICE.saturating_mul(u32::from(self.granted_time_extensions))
    }

    pub(super) fn progress_mark(&self) -> WorkflowProgressMark {
        let (committed_segments, authored_segments) = self
            .scope_plan
            .as_ref()
            .map_or((0, 0), |plan| (plan.committed_count(), plan.active));
        WorkflowProgressMark {
            committed_segments,
            authored_segments,
            valid_checks: self.valid_checks,
            retained_source_len: self.last_flowscript.as_deref().map_or(0, str::len),
            distinct_repair_states: self.flowscript_seen_repair_signatures.len(),
        }
    }

    /// Earn one more slice of wall clock, or refuse.
    ///
    /// The decision is made purely from the progress ledger — never from the model's own account of
    /// how well it is doing. The first grant compares against a zero mark, so a run that has not
    /// produced any source cannot buy time at all.
    pub(super) fn try_grant_time_extension(&mut self) -> TimeExtensionDecision {
        let earned = self.earned_wall_clock();
        if self.queued {
            return TimeExtensionDecision::NotExtendable;
        }
        let planned = scoped_wall_clock_extension(self.scope_plan.as_ref());
        if NESTED_RUN_WALL_CLOCK_BUDGET
            .saturating_add(planned)
            .saturating_add(earned)
            .saturating_add(EXTERNAL_TIME_EXTENSION_SLICE)
            > MAX_EXTERNAL_EARNED_WALL_CLOCK
        {
            return TimeExtensionDecision::CeilingReached { earned };
        }

        let mark = self.progress_mark();
        let previous = self.last_extension_progress.clone().unwrap_or_default();
        if !mark.advanced_beyond(&previous) {
            return TimeExtensionDecision::NoProgress { earned };
        }

        self.granted_time_extensions = self.granted_time_extensions.saturating_add(1);
        self.last_extension_progress = Some(mark);
        let grants = self.granted_time_extensions;
        let earned = self.earned_wall_clock();
        let elapsed_ms = self.shared_session_elapsed_ms();
        if let Some(session) = self.shared_session.as_mut() {
            let _ = session.record_time_extension(grants, earned.as_secs(), elapsed_ms);
        }
        TimeExtensionDecision::Granted { earned, grants }
    }

    /// Record an accepted plan. Returns the payload the model reads back.
    pub(super) fn accept_scope_plan_args(
        &mut self,
        args: PlanBoardScopeArgs,
    ) -> Result<serde_json::Value, ScopePlanRejection> {
        let accepted = match self.scope_plan.as_ref() {
            Some(existing) => existing.revise(args)?,
            None => accept_scope_plan(args)?,
        };
        let elapsed_ms = self.shared_session_elapsed_ms();
        if let Some(session) = self.shared_session.as_mut() {
            let _ = session.record_scope_plan(
                match accepted.strategy {
                    ScopeStrategy::Single => "single",
                    ScopeStrategy::Staged => "staged",
                    ScopeStrategy::Incremental => "incremental",
                    ScopeStrategy::MultiBoard => "multi_board",
                },
                accepted.segment_count(),
                accepted.revisions,
                elapsed_ms,
            );
        }
        let payload = accepted.acceptance_payload();
        self.scope_plan = Some(accepted);
        Ok(payload)
    }

    /// Return the existing plan when a restarted provider repeats the same execution shape.
    ///
    /// External continuations run in fresh processes. Even though the continuation prompt carries
    /// the accepted plan, a provider may defensively submit it again. That replay must be
    /// idempotent: consuming the one revision allowance here would turn a harmless process
    /// boundary into `SCOPE_PLAN_BUDGET_EXHAUSTED` before the first source write. Rationale is
    /// deliberately excluded because it is descriptive; strategy plus ordered segments are the
    /// executable identity.
    pub(super) fn repeated_scope_plan_payload(
        &self,
        args: &PlanBoardScopeArgs,
    ) -> Option<serde_json::Value> {
        let existing = self.scope_plan.as_ref()?;
        let proposed = accept_scope_plan(args.clone()).ok()?;
        if existing.strategy != proposed.strategy
            || serde_json::to_value(&existing.segments).ok()
                != serde_json::to_value(&proposed.segments).ok()
        {
            return None;
        }

        let mut payload = existing.acceptance_payload();
        if let Some(object) = payload.as_object_mut() {
            object.insert("idempotent".to_string(), serde_json::Value::Bool(true));
            object.insert(
                "message".to_string(),
                serde_json::Value::String(
                    "This exact scope plan is already accepted. Its host-owned active and committed state was preserved; do not call plan_board_scope again. Continue with the returned active segment and next_action."
                        .to_string(),
                ),
            );
        }
        Some(payload)
    }

    /// A staged plan grows one draft, so the host cannot see segment boundaries in the source. Each
    /// revision that checks `valid` is one more segment landed; that is what drives the remaining
    /// count reported to the caller and the prefix-commit decision below.
    pub(super) fn record_staged_segment_validated(&mut self) {
        if let Some(plan) = self.scope_plan.as_mut()
            && plan.strategy == ScopeStrategy::Staged
        {
            plan.mark_active_authored();
        }
    }

    /// Ask a staged plan to commit the prefix it already validated instead of growing until the
    /// wall clock kills every segment. Only applies to a multi-segment staged plan that has
    /// validated at least one segment and has not queued anything yet.
    pub(super) fn request_staged_prefix_commit(&mut self) -> bool {
        if self.queued {
            return false;
        }
        let Some(plan) = self.scope_plan.as_ref() else {
            return false;
        };
        if plan.strategy != ScopeStrategy::Staged
            || !plan.is_multi_segment()
            || plan.active == 0
            || plan.is_complete()
        {
            return false;
        }
        self.staged_prefix_commit_requested = true;
        true
    }

    /// A queued commit is real progress: every segment authored so far reached the board, and the
    /// retry lease is renewed so the next segment does not start against a stale zero-progress
    /// circuit.
    pub(super) fn record_scope_plan_commit(&mut self) {
        let elapsed_ms = self.shared_session_elapsed_ms();
        let Some(plan) = self.scope_plan.as_mut() else {
            return;
        };
        // A per-segment strategy commits the active segment; a staged one commits everything it
        // grew into the draft.
        if plan.strategy.commits_per_segment() {
            plan.mark_active_authored();
        } else {
            plan.active = plan.segment_count();
        }
        plan.mark_authored_committed();
        let completed = plan
            .segments
            .get(plan.active.saturating_sub(1))
            .map(|segment| (segment.id.clone(), plan.active, plan.segment_count()));
        if let Some((segment_id, index, total)) = completed
            && let Some(session) = self.shared_session.as_mut()
        {
            let _ = session.record_scope_segment_completed(&segment_id, index, total, elapsed_ms);
        }
    }

    pub(super) fn finish_interrupted_phase(&mut self) {
        if !self.edit_in_flight {
            return;
        }
        self.edit_in_flight = false;
        let message = match self.mutation_path {
            Some(WorkflowMutationPath::FlowScript) if self.flowscript_draft_retained => format!(
                "The provider or transport interrupted a FlowScript draft operation. Retained draft {} remains resumable at revision {}; continue that exact source revision in the next phase.",
                self.flowscript_draft_id.as_deref().unwrap_or("<unknown>"),
                self.flowscript_revision
                    .map(|revision| revision.to_string())
                    .unwrap_or_else(|| "<unknown>".to_string())
            ),
            Some(WorkflowMutationPath::TypedIr) if self.typed_draft_retained => format!(
                "The provider or transport interrupted a typed-IR operation. The retained draft {} remains resumable at revision {}; continue from that exact revision in the next phase.",
                self.typed_draft_id.as_deref().unwrap_or("<unknown>"),
                self.typed_revision
                    .map(|revision| revision.to_string())
                    .unwrap_or_else(|| "<unknown>".to_string())
            ),
            Some(WorkflowMutationPath::TypedIr) => "The provider or transport interrupted a typed-IR operation before draft retention could be confirmed. Do not claim a resumable draft unless the next tool response or host recovery context supplies its revision.".to_string(),
            Some(WorkflowMutationPath::FlowScript) => "The provider or transport interrupted a FlowScript operation before retained source could be confirmed. Do not claim a resumable draft unless the next tool response or host recovery context supplies its revision.".to_string(),
            _ => "The provider or transport interrupted the edit validator. The complete submitted draft was retained and must be resubmitted before any reduced fallback.".to_string(),
        };
        if let Some(interrupted) = self.in_flight_flowscript.take() {
            if self.repair_tracker.record_failed(&interrupted) {
                self.best_failed_errors = vec![message.clone()];
                self.candidate_regression_warning = None;
            }
            self.last_flowscript = Some(interrupted);
        }
        self.last_status = Some("edit_interrupted".to_string());
        self.last_errors = vec![message];
        self.pending_modular_fallback = None;
        self.declarations_since_edit = 0;
    }

    /// Name the loop budget that would refuse further source work on arrival, if any. `None`
    /// means the next phase can still dispatch operations.
    pub(super) fn exhausted_budget(&self) -> Option<String> {
        if self.queued {
            return None;
        }
        if let Some(circuit) = self
            .shared_session
            .as_ref()
            .map(|session| session.snapshot(self.shared_session_elapsed_ms()))
            .and_then(|snapshot| snapshot.circuit)
        {
            return Some(format!(
                "shared zero-progress circuit ({:?}, {} attempts)",
                circuit.reason, circuit.consecutive_zero_progress_attempts
            ));
        }
        if self.stalled_edit_attempts >= MAX_EXTERNAL_WORKFLOW_STALLED_EDIT_ATTEMPTS {
            return Some(format!(
                "stalled repair progress ({}/{} repeated compiler states)",
                self.stalled_edit_attempts, MAX_EXTERNAL_WORKFLOW_STALLED_EDIT_ATTEMPTS
            ));
        }
        let operation_budget = self.flowscript_operation_budget();
        if self.flowscript_operation_attempts >= operation_budget {
            return Some(format!(
                "FlowScript source operation budget ({}/{})",
                self.flowscript_operation_attempts, operation_budget
            ));
        }
        let edit_budget = self.edit_attempt_budget();
        if self.edit_attempts >= edit_budget {
            return Some(format!(
                "FlowScript check budget ({}/{})",
                self.edit_attempts, edit_budget
            ));
        }
        let commit_budget = self.commit_attempt_budget();
        if self.flowscript_commit_attempts >= commit_budget {
            return Some(format!(
                "commit retry budget ({}/{})",
                self.flowscript_commit_attempts, commit_budget
            ));
        }
        if self.typed_stalled_attempts >= MAX_EXTERNAL_TYPED_IR_STALLED_ATTEMPTS {
            return Some(format!(
                "typed-IR stalled repair progress ({}/{})",
                self.typed_stalled_attempts, MAX_EXTERNAL_TYPED_IR_STALLED_ATTEMPTS
            ));
        }
        let typed_budget = typed_ir_operation_budget(self.typed_expected_modules);
        if self.mutation_path == Some(WorkflowMutationPath::TypedIr)
            && self.typed_operation_attempts >= typed_budget
        {
            return Some(format!(
                "typed-IR operation budget ({}/{})",
                self.typed_operation_attempts, typed_budget
            ));
        }
        None
    }

    /// Make one host-granted continuation phase executable again. The stall detector restarts
    /// from a clean signature set and exhausted counters receive a small bounded headroom, so the
    /// continuation instructions ("repair the retained draft") are not refused-on-arrival by the
    /// budgets the previous phase burned.
    pub(super) fn grant_continuation_slice(&mut self) {
        if self.queued {
            return;
        }
        let elapsed_ms = self.shared_session_elapsed_ms();
        if let Some(session) = self.shared_session.as_mut() {
            let _ = session.begin_continuation(elapsed_ms);
        }
        self.stalled_edit_attempts = 0;
        self.flowscript_seen_repair_signatures.clear();
        self.typed_stalled_attempts = 0;
        self.typed_seen_repair_signatures.clear();
        let operation_budget = self.flowscript_operation_budget();
        if self.flowscript_operation_attempts >= operation_budget {
            self.flowscript_operation_attempts =
                operation_budget.saturating_sub(EXTERNAL_CONTINUATION_OPERATION_HEADROOM);
        }
        let edit_budget = self.edit_attempt_budget();
        if self.edit_attempts >= edit_budget {
            self.edit_attempts = edit_budget.saturating_sub(EXTERNAL_CONTINUATION_CHECK_HEADROOM);
        }
        let commit_budget = self.commit_attempt_budget();
        if self.flowscript_commit_attempts >= commit_budget {
            self.flowscript_commit_attempts = commit_budget.saturating_sub(1);
        }
        let typed_budget = typed_ir_operation_budget(self.typed_expected_modules);
        if self.typed_operation_attempts >= typed_budget {
            self.typed_operation_attempts =
                typed_budget.saturating_sub(EXTERNAL_CONTINUATION_OPERATION_HEADROOM);
        }
    }

    pub(super) fn record_flowscript_repair_progress(
        &mut self,
        status: Option<&str>,
        diagnostics: &[String],
        requires_repair: bool,
    ) {
        if !requires_repair {
            self.stalled_edit_attempts = 0;
            if matches!(status, Some("valid" | "queued" | "already_queued")) {
                self.flowscript_seen_repair_signatures.clear();
            }
            return;
        }

        let fingerprint =
            flowscript_repair_fingerprint(status, diagnostics, &self.last_structured_diagnostics);
        if self.flowscript_seen_repair_signatures.insert(fingerprint) {
            self.stalled_edit_attempts = 0;
        } else {
            self.stalled_edit_attempts = self.stalled_edit_attempts.saturating_add(1);
        }
        self.declarations_since_edit = 0;
    }
}
