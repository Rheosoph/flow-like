//! Pure workflow-session policy shared by every FlowPilot model transport.
//!
//! The session intentionally performs no model calls, filesystem writes, board mutations, or
//! wall-clock reads. Hosts supply monotonic elapsed milliseconds and observed results. This keeps
//! lifecycle decisions replayable across the built-in model loop, GitHub Copilot SDK, and MCP
//! code-agent adapters.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::manifest::BoardContextManifest;

pub const WORKFLOW_SESSION_SNAPSHOT_VERSION: &str = "flowpilot.workflow-session/v1";
const CONTEXT_READ_KEY_DOMAIN: &[u8] = b"flowpilot.workflow-session.context-read/v1\0";
const STRATEGY_FINGERPRINT_DOMAIN: &[u8] = b"flowpilot.workflow-session.strategy/v1\0";
const TELEMETRY_CHAIN_DOMAIN: &[u8] = b"flowpilot.workflow-session.telemetry-chain/v1\0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct WorkflowSessionPolicy {
    /// Deadline from session start to the first host-retained authoring artifact.
    pub first_artifact_sla_ms: u64,
    /// Unique DB/UI/storage/extension reads allowed before an artifact exists. Duplicate keys do
    /// not consume this budget and are rejected as duplicates.
    pub max_predraft_context_reads: u16,
    /// Total consecutive strategies that may produce no new host-observed progress.
    pub max_zero_progress_attempts: u16,
    /// Attempts with the same normalized strategy fingerprint before the circuit opens.
    pub max_same_strategy_attempts: u16,
    /// Number of latest telemetry events retained verbatim. Older events are synchronously
    /// compacted into the hash chain, counters, and first/last milestones.
    pub telemetry_event_capacity: usize,
}

impl Default for WorkflowSessionPolicy {
    fn default() -> Self {
        Self {
            first_artifact_sla_ms: 90_000,
            max_predraft_context_reads: 6,
            max_zero_progress_attempts: 2,
            max_same_strategy_attempts: 2,
            telemetry_event_capacity: 128,
        }
    }
}

impl WorkflowSessionPolicy {
    fn normalized(mut self) -> Self {
        self.max_zero_progress_attempts = self.max_zero_progress_attempts.max(1);
        self.max_same_strategy_attempts = self.max_same_strategy_attempts.max(1);
        self.telemetry_event_capacity = self.telemetry_event_capacity.max(1);
        self
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowSessionPhase {
    Initialized,
    ContextReady,
    Discovering,
    Authoring,
    Validating,
    Validated,
    Prepared,
    AwaitingApproval,
    Applying,
    Applied,
    Dismissed,
    Failed,
    Cancelled,
}

impl WorkflowSessionPhase {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Applied | Self::Dismissed | Self::Failed | Self::Cancelled
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case", tag = "domain", content = "name")]
pub enum ContextReadDomain {
    Board,
    Catalog,
    Declarations,
    Database,
    Ui,
    Storage,
    Extension(String),
}

impl ContextReadDomain {
    fn consumes_predraft_budget(&self) -> bool {
        matches!(
            self,
            Self::Database | Self::Ui | Self::Storage | Self::Extension(_)
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct ContextReadKey {
    pub domain: ContextReadDomain,
    pub operation: String,
    pub selector_fingerprint: String,
}

impl ContextReadKey {
    pub fn new(domain: ContextReadDomain, operation: &str, selector: &Value) -> Self {
        let operation = normalize_words(operation);
        let selector = canonicalize_json(selector.clone());
        let material = serde_json::to_vec(&(&domain, &operation, selector)).unwrap_or_default();
        Self {
            domain,
            operation,
            selector_fingerprint: domain_hash(CONTEXT_READ_KEY_DOMAIN, &material),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "decision")]
pub enum ContextReadDecision {
    Accepted {
        key: ContextReadKey,
        predraft_unique_reads: u16,
        predraft_reads_remaining: u16,
    },
    Duplicate {
        key: ContextReadKey,
        predraft_unique_reads: u16,
    },
    PredraftBudgetExhausted {
        key: ContextReadKey,
        limit: u16,
        next_action: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "decision")]
pub enum WorkflowToolPreflightDecision {
    Dispatch {
        /// Exact ownership token for a reserved context read. Completion and abort must return
        /// this token; matching only by selector lets a late attempt consume a newer retry's
        /// reservation.
        lease: Option<WorkflowToolLease>,
    },
    ShortCircuit {
        status: String,
        code: String,
        retryable: bool,
        next_action: String,
        message: String,
    },
}

impl WorkflowToolPreflightDecision {
    pub fn lease(&self) -> Option<&WorkflowToolLease> {
        match self {
            Self::Dispatch { lease } => lease.as_ref(),
            Self::ShortCircuit { .. } => None,
        }
    }

    pub fn short_circuit_result(&self) -> Option<Value> {
        match self {
            Self::Dispatch { .. } => None,
            Self::ShortCircuit {
                status,
                code,
                retryable,
                next_action,
                message,
            } => Some(json!({
                "status": status,
                "code": code,
                "retryable": retryable,
                "next_action": next_action,
                "message": message,
            })),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowToolLease {
    reservation_id: u64,
    key: ContextReadKey,
}

#[derive(Debug, Clone)]
struct InFlightContextRead {
    reservation_id: u64,
    consumes_predraft_budget: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowArtifactKind {
    FlowScript,
    TypedIr,
    DirectCommands,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowArtifactState {
    pub kind: WorkflowArtifactKind,
    pub artifact_id: String,
    pub revision: u64,
    pub digest: String,
    pub first_retained_at_ms: u64,
    pub last_retained_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum FirstArtifactSlaStatus {
    Pending { remaining_ms: u64 },
    Breached { overdue_ms: u64 },
    Satisfied { first_retained_at_ms: u64 },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowValidationStatus {
    Valid,
    Invalid,
    Interrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowValidationState {
    pub status: WorkflowValidationStatus,
    pub artifact_revision: u64,
    pub diagnostic_fingerprint: Option<String>,
    pub completed_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PreparedWorkflowState {
    pub review_id: String,
    pub artifact_revision: u64,
    pub prepared_at_ms: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum CircuitOpenReason {
    RepeatedStrategy,
    ZeroProgressBudgetExhausted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowCircuitState {
    pub reason: CircuitOpenReason,
    pub opened_at_ms: u64,
    pub strategy_fingerprint: String,
    pub consecutive_zero_progress_attempts: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "decision")]
pub enum StrategyDecision {
    Progressed {
        strategy_fingerprint: String,
    },
    RetryWithDifferentStrategy {
        strategy_fingerprint: String,
        zero_progress_attempts: u16,
        attempts_remaining: u16,
    },
    CircuitOpen {
        state: WorkflowCircuitState,
    },
}

/// Provider-neutral outcome of feeding one completed tool call into the shared workflow policy.
/// Adapters keep responsibility for transport and tool execution; artifact, validation, retry,
/// and review semantics are interpreted once here for Bits, Copilot SDK, and MCP code agents.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct WorkflowToolObservation {
    pub context_read: Option<ContextReadDecision>,
    pub artifact_retained: bool,
    pub validation_recorded: bool,
    pub review_prepared: bool,
    pub strategy_decision: Option<StrategyDecision>,
}

impl WorkflowToolObservation {
    pub fn circuit_open(&self) -> bool {
        matches!(
            self.strategy_decision,
            Some(StrategyDecision::CircuitOpen { .. })
        )
    }
}

/// Stable fingerprint for a structured retry strategy. Objects are key-sorted; array order stays
/// meaningful because it often represents an ordered repair plan.
pub fn workflow_strategy_fingerprint(strategy: &Value) -> String {
    let value = canonicalize_json(strategy.clone());
    domain_hash(
        STRATEGY_FINGERPRINT_DOMAIN,
        &serde_json::to_vec(&value).unwrap_or_default(),
    )
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowTelemetryKind {
    SessionStarted,
    ManifestReady,
    DiscoveryStarted,
    ContextRead,
    ContextReadReserved,
    ContextReadAborted,
    ContextReadDeduplicated,
    PredraftContextBudgetExhausted,
    FirstArtifactSlaBreached,
    ArtifactRetained,
    ValidationCompleted,
    StrategyAttempted,
    CircuitOpened,
    ReviewPrepared,
    ApprovalRequested,
    ApplyStarted,
    Applied,
    Dismissed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowTelemetryEvent {
    pub sequence: u64,
    pub elapsed_ms: u64,
    pub kind: WorkflowTelemetryKind,
    pub payload: Value,
    pub chain_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowTelemetryMilestone {
    pub first: WorkflowTelemetryEvent,
    pub last: WorkflowTelemetryEvent,
}

/// Bounded, synchronous telemetry with no silent drop path. Every appended event advances the
/// hash chain and counters. Once the verbatim window fills, older events are compacted rather than
/// discarded without evidence; first/last events of every finite event kind remain available.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowTelemetryLedger {
    capacity: usize,
    total_events: u64,
    compacted_events: u64,
    chain_digest: String,
    kind_counts: BTreeMap<WorkflowTelemetryKind, u64>,
    milestones: BTreeMap<WorkflowTelemetryKind, WorkflowTelemetryMilestone>,
    recent_events: VecDeque<WorkflowTelemetryEvent>,
}

impl WorkflowTelemetryLedger {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            total_events: 0,
            compacted_events: 0,
            chain_digest: domain_hash(TELEMETRY_CHAIN_DOMAIN, b"genesis"),
            kind_counts: BTreeMap::new(),
            milestones: BTreeMap::new(),
            recent_events: VecDeque::new(),
        }
    }

    pub fn append(&mut self, elapsed_ms: u64, kind: WorkflowTelemetryKind, payload: Value) {
        let sequence = self.total_events;
        let payload = canonicalize_json(payload);
        #[derive(Serialize)]
        struct ChainMaterial<'a> {
            previous: &'a str,
            sequence: u64,
            elapsed_ms: u64,
            kind: WorkflowTelemetryKind,
            payload: &'a Value,
        }
        let material = ChainMaterial {
            previous: &self.chain_digest,
            sequence,
            elapsed_ms,
            kind,
            payload: &payload,
        };
        let bytes = serde_json::to_vec(&material).unwrap_or_default();
        let event = WorkflowTelemetryEvent {
            sequence,
            elapsed_ms,
            kind,
            payload,
            chain_digest: domain_hash(TELEMETRY_CHAIN_DOMAIN, &bytes),
        };

        self.total_events = self.total_events.saturating_add(1);
        self.chain_digest.clone_from(&event.chain_digest);
        *self.kind_counts.entry(kind).or_default() += 1;
        self.milestones
            .entry(kind)
            .and_modify(|milestone| milestone.last = event.clone())
            .or_insert_with(|| WorkflowTelemetryMilestone {
                first: event.clone(),
                last: event.clone(),
            });
        if self.recent_events.len() == self.capacity {
            self.recent_events.pop_front();
            self.compacted_events = self.compacted_events.saturating_add(1);
        }
        self.recent_events.push_back(event);
    }

    pub fn total_events(&self) -> u64 {
        self.total_events
    }

    pub fn compacted_events(&self) -> u64 {
        self.compacted_events
    }

    pub fn chain_digest(&self) -> &str {
        &self.chain_digest
    }

    pub fn count(&self, kind: WorkflowTelemetryKind) -> u64 {
        self.kind_counts.get(&kind).copied().unwrap_or_default()
    }

    pub fn recent_events(&self) -> &VecDeque<WorkflowTelemetryEvent> {
        &self.recent_events
    }

    pub fn milestones(&self) -> &BTreeMap<WorkflowTelemetryKind, WorkflowTelemetryMilestone> {
        &self.milestones
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkflowSessionSnapshot {
    pub schema: String,
    pub manifest_fingerprint: String,
    pub policy: WorkflowSessionPolicy,
    pub phase: WorkflowSessionPhase,
    pub elapsed_ms: u64,
    pub first_artifact_sla: FirstArtifactSlaStatus,
    pub artifact: Option<WorkflowArtifactState>,
    pub validation: Option<WorkflowValidationState>,
    pub prepared: Option<PreparedWorkflowState>,
    pub context_reads: Vec<ContextReadKey>,
    pub in_flight_context_reads: Vec<ContextReadKey>,
    pub predraft_unique_context_reads: u16,
    pub strategy_attempts: BTreeMap<String, u16>,
    pub consecutive_zero_progress_attempts: u16,
    pub circuit: Option<WorkflowCircuitState>,
    pub terminal_reason: Option<String>,
    pub telemetry: WorkflowTelemetryLedger,
}

#[derive(Debug, Clone)]
pub struct WorkflowSession {
    manifest: BoardContextManifest,
    policy: WorkflowSessionPolicy,
    phase: WorkflowSessionPhase,
    artifact: Option<WorkflowArtifactState>,
    validation: Option<WorkflowValidationState>,
    prepared: Option<PreparedWorkflowState>,
    context_reads: BTreeSet<ContextReadKey>,
    /// Reads admitted by shared preflight but not yet proven successful. Reservations prevent
    /// concurrent budget oversubscription without poisoning exact retries after failure.
    in_flight_context_reads: BTreeMap<ContextReadKey, InFlightContextRead>,
    next_context_read_reservation_id: u64,
    predraft_unique_context_reads: u16,
    strategy_attempts: BTreeMap<String, u16>,
    consecutive_zero_progress_attempts: u16,
    last_progress_fingerprint: Option<String>,
    circuit: Option<WorkflowCircuitState>,
    first_artifact_sla_breach_recorded: bool,
    terminal_reason: Option<String>,
    telemetry: WorkflowTelemetryLedger,
}

impl WorkflowSession {
    pub fn new(manifest: BoardContextManifest, policy: WorkflowSessionPolicy) -> Self {
        let policy = policy.normalized();
        let mut telemetry = WorkflowTelemetryLedger::new(policy.telemetry_event_capacity);
        telemetry.append(
            0,
            WorkflowTelemetryKind::SessionStarted,
            json!({
                "manifest_fingerprint": manifest.fingerprint,
                "manifest_schema": manifest.schema,
            }),
        );
        Self {
            manifest,
            policy,
            phase: WorkflowSessionPhase::Initialized,
            artifact: None,
            validation: None,
            prepared: None,
            context_reads: BTreeSet::new(),
            in_flight_context_reads: BTreeMap::new(),
            next_context_read_reservation_id: 1,
            predraft_unique_context_reads: 0,
            strategy_attempts: BTreeMap::new(),
            consecutive_zero_progress_attempts: 0,
            last_progress_fingerprint: None,
            circuit: None,
            first_artifact_sla_breach_recorded: false,
            terminal_reason: None,
            telemetry,
        }
    }

    pub fn manifest(&self) -> &BoardContextManifest {
        &self.manifest
    }

    pub fn policy(&self) -> &WorkflowSessionPolicy {
        &self.policy
    }

    pub fn phase(&self) -> WorkflowSessionPhase {
        self.phase
    }

    pub fn telemetry(&self) -> &WorkflowTelemetryLedger {
        &self.telemetry
    }

    pub fn mark_manifest_ready(&mut self, elapsed_ms: u64) -> Result<(), WorkflowSessionError> {
        self.require_active("mark manifest ready")?;
        if self.phase == WorkflowSessionPhase::Initialized {
            self.phase = WorkflowSessionPhase::ContextReady;
            self.telemetry.append(
                elapsed_ms,
                WorkflowTelemetryKind::ManifestReady,
                json!({"manifest_fingerprint": self.manifest.fingerprint}),
            );
        }
        Ok(())
    }

    pub fn begin_discovery(&mut self, elapsed_ms: u64) -> Result<(), WorkflowSessionError> {
        self.require_active("begin discovery")?;
        if self.phase == WorkflowSessionPhase::Initialized {
            return Err(WorkflowSessionError::InvalidTransition {
                from: self.phase,
                operation: "begin discovery",
            });
        }
        if self.artifact.is_none() {
            self.phase = WorkflowSessionPhase::Discovering;
        }
        self.telemetry.append(
            elapsed_ms,
            WorkflowTelemetryKind::DiscoveryStarted,
            json!({}),
        );
        Ok(())
    }

    /// Start one host-bounded continuation after a circuit break. The retained artifact,
    /// validation, manifest, and telemetry survive; only the consecutive retry lease is renewed.
    /// Hosts decide how many continuations are allowed, keeping this pure policy provider-neutral.
    pub fn begin_continuation(&mut self, elapsed_ms: u64) -> Result<(), WorkflowSessionError> {
        self.require_active("begin continuation")?;
        self.consecutive_zero_progress_attempts = 0;
        self.strategy_attempts.clear();
        self.circuit = None;
        self.telemetry.append(
            elapsed_ms,
            WorkflowTelemetryKind::DiscoveryStarted,
            json!({ "continuation": true }),
        );
        Ok(())
    }

    pub fn record_context_read(
        &mut self,
        domain: ContextReadDomain,
        operation: &str,
        selector: &Value,
        elapsed_ms: u64,
    ) -> Result<ContextReadDecision, WorkflowSessionError> {
        let (decision, lease) =
            self.reserve_context_read(domain, operation, selector, elapsed_ms)?;
        if let Some(lease) = lease.as_ref() {
            self.finish_context_read(lease, true, elapsed_ms)?;
        }
        Ok(decision)
    }

    pub fn preflight_tool_call(
        &mut self,
        tool_name: &str,
        arguments: &Value,
        elapsed_ms: u64,
    ) -> Result<WorkflowToolPreflightDecision, WorkflowSessionError> {
        self.require_active("preflight tool call")?;
        let Some(domain) = tool_context_domain(tool_name) else {
            return Ok(WorkflowToolPreflightDecision::Dispatch { lease: None });
        };
        if !domain.consumes_predraft_budget() {
            return Ok(WorkflowToolPreflightDecision::Dispatch { lease: None });
        }

        if self.artifact.is_none()
            && manifest_covers_context_read(&self.manifest, tool_name, arguments)
        {
            return Ok(WorkflowToolPreflightDecision::ShortCircuit {
                status: "context_preloaded".to_string(),
                code: "CONTEXT_ALREADY_IN_MANIFEST".to_string(),
                // Retrying the identical call can never succeed; external CLIs treat
                // retryable:true as "retry this exact call" and spin on it.
                retryable: false,
                next_action: "write_flowscript".to_string(),
                message: "This exact inventory read is already present in the immutable authoring manifest. Reuse it and retain the full-shape FlowScript artifact now; the duplicate frontend call was not dispatched.".to_string(),
            });
        }
        if self.artifact.is_none()
            && matches!(
                self.observe_first_artifact_sla(elapsed_ms),
                FirstArtifactSlaStatus::Breached { .. }
            )
        {
            return Ok(WorkflowToolPreflightDecision::ShortCircuit {
                status: "first_artifact_sla_breached".to_string(),
                code: "FIRST_ARTIFACT_SLA_BREACHED".to_string(),
                retryable: false,
                next_action: "write_flowscript".to_string(),
                message: "The shared first-artifact deadline elapsed. Stop context discovery and retain the complete full-shape FlowScript source now; this inspection was not dispatched.".to_string(),
            });
        }

        let operation = context_read_operation(tool_name, arguments);
        match self.reserve_context_read(domain, operation, arguments, elapsed_ms)? {
            (ContextReadDecision::Accepted { .. }, lease) => {
                Ok(WorkflowToolPreflightDecision::Dispatch { lease })
            }
            (ContextReadDecision::Duplicate { .. }, _) => {
                Ok(WorkflowToolPreflightDecision::ShortCircuit {
                    status: "duplicate_context_read".to_string(),
                    code: "DUPLICATE_CONTEXT_READ".to_string(),
                    retryable: false,
                    next_action: if self.artifact.is_some() {
                        "repair_retained_artifact".to_string()
                    } else {
                        "write_flowscript".to_string()
                    },
                    message: "This exact context selector was already read or is still in flight in the shared session. Reuse its result; the duplicate call was not dispatched.".to_string(),
                })
            }
            (ContextReadDecision::PredraftBudgetExhausted { .. }, _) => {
                Ok(WorkflowToolPreflightDecision::ShortCircuit {
                    status: "predraft_inspection_budget_exhausted".to_string(),
                    code: "PREDRAFT_INSPECTION_BUDGET_EXHAUSTED".to_string(),
                    retryable: false,
                    next_action: "write_flowscript".to_string(),
                    message: "The shared pre-draft context budget is exhausted. Reuse prior facts and retain a full-shape artifact before any focused follow-up read.".to_string(),
                })
            }
        }
    }

    pub fn complete_tool_call(
        &mut self,
        lease: Option<&WorkflowToolLease>,
        tool_name: &str,
        arguments: &Value,
        result_text: &str,
        succeeded: bool,
        elapsed_ms: u64,
    ) -> Result<WorkflowToolObservation, WorkflowSessionError> {
        if let Some(domain) = tool_context_domain(tool_name) {
            let operation = context_read_operation(tool_name, arguments);
            let key = ContextReadKey::new(domain, operation, arguments);
            let decision = if let Some(lease) = lease.filter(|lease| lease.key == key) {
                self.finish_context_read(lease, succeeded, elapsed_ms)?
            } else if self.in_flight_context_reads.contains_key(&key) {
                // A completion without the exact lease may belong to an older timed-out attempt.
                // Never let it consume or abort the currently reserved retry.
                None
            } else if succeeded {
                Some(self.record_context_read(
                    key.domain.clone(),
                    operation,
                    arguments,
                    elapsed_ms,
                )?)
            } else {
                None
            };
            return Ok(WorkflowToolObservation {
                context_read: decision,
                ..WorkflowToolObservation::default()
            });
        }
        if !succeeded && !tool_result_proves_retained_artifact(tool_name, result_text) {
            return Ok(WorkflowToolObservation::default());
        }
        self.record_tool_result(tool_name, arguments, result_text, elapsed_ms)
    }

    pub fn abort_tool_call(
        &mut self,
        lease: Option<&WorkflowToolLease>,
        tool_name: &str,
        arguments: &Value,
        elapsed_ms: u64,
    ) -> Result<(), WorkflowSessionError> {
        self.require_active("abort tool call")?;
        let Some(domain) = tool_context_domain(tool_name) else {
            return Ok(());
        };
        let operation = context_read_operation(tool_name, arguments);
        let key = ContextReadKey::new(domain, operation, arguments);
        if let Some(lease) = lease.filter(|lease| lease.key == key) {
            let _ = self.finish_context_read(lease, false, elapsed_ms)?;
        }
        Ok(())
    }

    fn reserve_context_read(
        &mut self,
        domain: ContextReadDomain,
        operation: &str,
        selector: &Value,
        elapsed_ms: u64,
    ) -> Result<(ContextReadDecision, Option<WorkflowToolLease>), WorkflowSessionError> {
        self.require_active("reserve context read")?;
        let key = ContextReadKey::new(domain, operation, selector);
        if self.context_reads.contains(&key) || self.in_flight_context_reads.contains_key(&key) {
            self.telemetry.append(
                elapsed_ms,
                WorkflowTelemetryKind::ContextReadDeduplicated,
                json!({"key": key}),
            );
            return Ok((
                ContextReadDecision::Duplicate {
                    key,
                    predraft_unique_reads: self.predraft_unique_context_reads,
                },
                None,
            ));
        }

        let consumes_predraft_budget =
            self.artifact.is_none() && key.domain.consumes_predraft_budget();
        let reserved_predraft_reads = self
            .in_flight_context_reads
            .values()
            .filter(|reserved| reserved.consumes_predraft_budget)
            .count()
            .min(u16::MAX as usize) as u16;
        let admitted_predraft_reads = self
            .predraft_unique_context_reads
            .saturating_add(reserved_predraft_reads);
        if consumes_predraft_budget
            && admitted_predraft_reads >= self.policy.max_predraft_context_reads
        {
            self.telemetry.append(
                elapsed_ms,
                WorkflowTelemetryKind::PredraftContextBudgetExhausted,
                json!({
                    "key": key,
                    "limit": self.policy.max_predraft_context_reads,
                    "next_action": "retain_full_shape_artifact",
                }),
            );
            return Ok((
                ContextReadDecision::PredraftBudgetExhausted {
                    key,
                    limit: self.policy.max_predraft_context_reads,
                    next_action: "retain_full_shape_artifact".to_string(),
                },
                None,
            ));
        }

        let reservation_id = self.next_context_read_reservation_id;
        self.next_context_read_reservation_id = self
            .next_context_read_reservation_id
            .checked_add(1)
            .unwrap_or(1);
        self.in_flight_context_reads.insert(
            key.clone(),
            InFlightContextRead {
                reservation_id,
                consumes_predraft_budget,
            },
        );
        let admitted_predraft_reads =
            admitted_predraft_reads.saturating_add(u16::from(consumes_predraft_budget));
        let remaining = self
            .policy
            .max_predraft_context_reads
            .saturating_sub(admitted_predraft_reads);
        self.telemetry.append(
            elapsed_ms,
            WorkflowTelemetryKind::ContextReadReserved,
            json!({
                "key": key,
                "predraft_unique_reads": admitted_predraft_reads,
                "predraft_reads_remaining": remaining,
            }),
        );
        let lease = WorkflowToolLease {
            reservation_id,
            key: key.clone(),
        };
        Ok((
            ContextReadDecision::Accepted {
                key,
                predraft_unique_reads: admitted_predraft_reads,
                predraft_reads_remaining: remaining,
            },
            Some(lease),
        ))
    }

    fn finish_context_read(
        &mut self,
        lease: &WorkflowToolLease,
        succeeded: bool,
        elapsed_ms: u64,
    ) -> Result<Option<ContextReadDecision>, WorkflowSessionError> {
        self.require_active("finish context read")?;
        let Some(reservation) = self.in_flight_context_reads.get(&lease.key) else {
            return Ok(None);
        };
        if reservation.reservation_id != lease.reservation_id {
            return Ok(None);
        }
        let consumed_predraft_budget = reservation.consumes_predraft_budget;
        self.in_flight_context_reads.remove(&lease.key);
        if !succeeded {
            self.telemetry.append(
                elapsed_ms,
                WorkflowTelemetryKind::ContextReadAborted,
                json!({"key": lease.key, "reservation_id": lease.reservation_id}),
            );
            return Ok(None);
        }
        self.context_reads.insert(lease.key.clone());
        if consumed_predraft_budget {
            self.predraft_unique_context_reads =
                self.predraft_unique_context_reads.saturating_add(1);
        }
        let remaining = self
            .policy
            .max_predraft_context_reads
            .saturating_sub(self.predraft_unique_context_reads);
        self.telemetry.append(
            elapsed_ms,
            WorkflowTelemetryKind::ContextRead,
            json!({
                "key": lease.key,
                "reservation_id": lease.reservation_id,
                "predraft_unique_reads": self.predraft_unique_context_reads,
                "predraft_reads_remaining": remaining,
            }),
        );
        Ok(Some(ContextReadDecision::Accepted {
            key: lease.key.clone(),
            predraft_unique_reads: self.predraft_unique_context_reads,
            predraft_reads_remaining: remaining,
        }))
    }

    pub fn first_artifact_sla_status(&self, elapsed_ms: u64) -> FirstArtifactSlaStatus {
        if let Some(artifact) = &self.artifact {
            return FirstArtifactSlaStatus::Satisfied {
                first_retained_at_ms: artifact.first_retained_at_ms,
            };
        }
        if elapsed_ms > self.policy.first_artifact_sla_ms {
            FirstArtifactSlaStatus::Breached {
                overdue_ms: elapsed_ms.saturating_sub(self.policy.first_artifact_sla_ms),
            }
        } else {
            FirstArtifactSlaStatus::Pending {
                remaining_ms: self.policy.first_artifact_sla_ms.saturating_sub(elapsed_ms),
            }
        }
    }

    /// Evaluate the SLA and synchronously retain the first breach event. Calling this repeatedly
    /// never duplicates the causal milestone.
    pub fn observe_first_artifact_sla(&mut self, elapsed_ms: u64) -> FirstArtifactSlaStatus {
        let status = self.first_artifact_sla_status(elapsed_ms);
        if let FirstArtifactSlaStatus::Breached { overdue_ms } = status
            && !self.first_artifact_sla_breach_recorded
        {
            self.first_artifact_sla_breach_recorded = true;
            self.telemetry.append(
                elapsed_ms,
                WorkflowTelemetryKind::FirstArtifactSlaBreached,
                json!({
                    "deadline_ms": self.policy.first_artifact_sla_ms,
                    "overdue_ms": overdue_ms,
                    "next_action": "retain_full_shape_artifact",
                }),
            );
        }
        status
    }

    pub fn record_artifact(
        &mut self,
        kind: WorkflowArtifactKind,
        artifact_id: impl Into<String>,
        revision: u64,
        digest: impl Into<String>,
        elapsed_ms: u64,
    ) -> Result<(), WorkflowSessionError> {
        self.require_active("retain artifact")?;
        let artifact_id = artifact_id.into();
        let digest = digest.into();
        // A different artifact id is a legitimate rebinding: the host's own stale-draft recovery
        // instructs a fresh draft_id, and the draft store already enforces real identity rules.
        // A same-id lower revision is a salvage commit of an older checked revision — keep the
        // newer session state and record nothing rather than failing the whole session (which the
        // callers coerce into a run-terminating circuit-open).
        if let Some(existing) = &self.artifact
            && existing.artifact_id == artifact_id
            && revision < existing.revision
        {
            return Ok(());
        }
        let artifact_progressed = self.artifact.as_ref().is_none_or(|existing| {
            existing.kind != kind
                || existing.artifact_id != artifact_id
                || existing.revision != revision
                || existing.digest != digest
        });
        let first_retained_at_ms = self
            .artifact
            .as_ref()
            .map(|artifact| artifact.first_retained_at_ms)
            .unwrap_or(elapsed_ms);
        self.artifact = Some(WorkflowArtifactState {
            kind,
            artifact_id,
            revision,
            digest: digest.clone(),
            first_retained_at_ms,
            last_retained_at_ms: elapsed_ms,
        });
        self.validation = None;
        self.prepared = None;
        self.phase = WorkflowSessionPhase::Authoring;
        if artifact_progressed {
            self.note_progress(digest);
        }
        self.telemetry.append(
            elapsed_ms,
            WorkflowTelemetryKind::ArtifactRetained,
            json!({
                "kind": kind,
                "artifact_id": self.artifact.as_ref().map(|artifact| &artifact.artifact_id),
                "revision": revision,
                "first_retained_at_ms": first_retained_at_ms,
            }),
        );
        Ok(())
    }

    pub fn record_validation(
        &mut self,
        status: WorkflowValidationStatus,
        artifact_revision: u64,
        diagnostic_fingerprint: Option<String>,
        elapsed_ms: u64,
    ) -> Result<(), WorkflowSessionError> {
        self.require_active("record validation")?;
        let artifact = self
            .artifact
            .as_ref()
            .ok_or(WorkflowSessionError::ArtifactRequired("record validation"))?;
        if artifact.revision != artifact_revision {
            return Err(WorkflowSessionError::RevisionMismatch {
                expected: artifact.revision,
                received: artifact_revision,
            });
        }
        self.phase = WorkflowSessionPhase::Validating;
        self.validation = Some(WorkflowValidationState {
            status,
            artifact_revision,
            diagnostic_fingerprint: diagnostic_fingerprint.clone(),
            completed_at_ms: elapsed_ms,
        });
        self.prepared = None;
        self.phase = if status == WorkflowValidationStatus::Valid {
            WorkflowSessionPhase::Validated
        } else {
            WorkflowSessionPhase::Authoring
        };
        self.telemetry.append(
            elapsed_ms,
            WorkflowTelemetryKind::ValidationCompleted,
            json!({
                "status": status,
                "artifact_revision": artifact_revision,
                "diagnostic_fingerprint": diagnostic_fingerprint,
            }),
        );
        Ok(())
    }

    pub fn record_strategy_attempt(
        &mut self,
        strategy: &Value,
        progress_fingerprint: Option<&str>,
        elapsed_ms: u64,
    ) -> Result<StrategyDecision, WorkflowSessionError> {
        self.require_active("record strategy attempt")?;
        let strategy_fingerprint = workflow_strategy_fingerprint(strategy);
        let progressed = progress_fingerprint
            .filter(|fingerprint| !fingerprint.trim().is_empty())
            .is_some_and(|fingerprint| {
                self.last_progress_fingerprint.as_deref() != Some(fingerprint)
            });
        if progressed {
            self.note_progress(progress_fingerprint.unwrap_or_default().to_string());
            self.telemetry.append(
                elapsed_ms,
                WorkflowTelemetryKind::StrategyAttempted,
                json!({
                    "strategy_fingerprint": strategy_fingerprint,
                    "progressed": true,
                }),
            );
            return Ok(StrategyDecision::Progressed {
                strategy_fingerprint,
            });
        }

        let attempts = self
            .strategy_attempts
            .entry(strategy_fingerprint.clone())
            .or_default();
        *attempts = attempts.saturating_add(1);
        let same_strategy_attempts = *attempts;
        self.consecutive_zero_progress_attempts =
            self.consecutive_zero_progress_attempts.saturating_add(1);
        let reason = if same_strategy_attempts >= self.policy.max_same_strategy_attempts {
            Some(CircuitOpenReason::RepeatedStrategy)
        } else if self.consecutive_zero_progress_attempts >= self.policy.max_zero_progress_attempts
        {
            Some(CircuitOpenReason::ZeroProgressBudgetExhausted)
        } else {
            None
        };
        self.telemetry.append(
            elapsed_ms,
            WorkflowTelemetryKind::StrategyAttempted,
            json!({
                "strategy_fingerprint": strategy_fingerprint,
                "progressed": false,
                "same_strategy_attempts": same_strategy_attempts,
                "consecutive_zero_progress_attempts": self.consecutive_zero_progress_attempts,
            }),
        );
        if let Some(reason) = reason {
            let state = WorkflowCircuitState {
                reason,
                opened_at_ms: elapsed_ms,
                strategy_fingerprint,
                consecutive_zero_progress_attempts: self.consecutive_zero_progress_attempts,
            };
            self.circuit = Some(state.clone());
            self.telemetry.append(
                elapsed_ms,
                WorkflowTelemetryKind::CircuitOpened,
                json!({"state": state}),
            );
            return Ok(StrategyDecision::CircuitOpen { state });
        }

        Ok(StrategyDecision::RetryWithDifferentStrategy {
            strategy_fingerprint,
            zero_progress_attempts: self.consecutive_zero_progress_attempts,
            attempts_remaining: self
                .policy
                .max_zero_progress_attempts
                .saturating_sub(self.consecutive_zero_progress_attempts),
        })
    }

    /// Interpret a completed workflow tool call using one backend-independent lifecycle. The
    /// method deliberately accepts plain JSON/text so every adapter can call it without importing
    /// a provider SDK. Unknown/read-only tools are harmless no-ops.
    pub fn record_tool_result(
        &mut self,
        tool_name: &str,
        arguments: &Value,
        result_text: &str,
        elapsed_ms: u64,
    ) -> Result<WorkflowToolObservation, WorkflowSessionError> {
        if tool_context_domain(tool_name).is_some() {
            return self.complete_tool_call(
                None,
                tool_name,
                arguments,
                result_text,
                true,
                elapsed_ms,
            );
        }

        let artifact_kind = match tool_name {
            "edit_flowscript" | "write_flowscript" | "patch_flowscript" | "check_flowscript"
            | "commit_flowscript" => WorkflowArtifactKind::FlowScript,
            "begin_flow_ir_draft"
            | "update_flow_ir_draft"
            | "upsert_flow_ir_module"
            | "validate_flow_ir_draft"
            | "commit_flow_ir_draft" => WorkflowArtifactKind::TypedIr,
            "emit_commands" if result_text.contains("<commands>") => {
                WorkflowArtifactKind::DirectCommands
            }
            _ => return Ok(WorkflowToolObservation::default()),
        };

        let parsed = parse_tool_result_value(result_text);
        let status = parsed
            .as_ref()
            .and_then(|value| value.get("status"))
            .and_then(Value::as_str)
            .or_else(|| {
                (artifact_kind == WorkflowArtifactKind::DirectCommands).then_some("queued")
            });
        let rejected_identity = matches!(status, Some("request_identity_mismatch"))
            || parsed
                .as_ref()
                .and_then(|value| value.get("code"))
                .and_then(Value::as_str)
                .is_some_and(|code| {
                    matches!(
                        code,
                        "FLOWSCRIPT_DRAFT_MISSING" | "FLOWSCRIPT_BASE_REVISION_CONFLICT"
                    )
                });

        let previous_artifact = self.artifact.clone();
        let source = parsed
            .as_ref()
            .and_then(|value| value.get("source").or_else(|| value.get("flowscript")))
            .and_then(Value::as_str)
            .or_else(|| source_argument(arguments));
        let explicit_draft_id = parsed
            .as_ref()
            .and_then(|value| value.get("draft_id"))
            .and_then(Value::as_str)
            .or_else(|| arguments.get("draft_id").and_then(Value::as_str));
        let explicit_revision = parsed
            .as_ref()
            .and_then(|value| value.get("revision"))
            .and_then(Value::as_u64)
            .or_else(|| {
                ["expected_revision", "base_revision", "revision"]
                    .into_iter()
                    .find_map(|key| arguments.get(key).and_then(Value::as_u64))
            });

        let candidate_digest = match artifact_kind {
            WorkflowArtifactKind::FlowScript => source
                .map(|source| workflow_strategy_fingerprint(&json!({ "source": source })))
                .or_else(|| {
                    previous_artifact
                        .as_ref()
                        .filter(|artifact| artifact.kind == artifact_kind)
                        .map(|artifact| artifact.digest.clone())
                }),
            WorkflowArtifactKind::TypedIr => Some(workflow_strategy_fingerprint(&json!({
                "draft_id": explicit_draft_id,
                "revision": explicit_revision,
                "flowscript": source,
                "result": parsed,
            }))),
            WorkflowArtifactKind::DirectCommands => Some(workflow_strategy_fingerprint(&json!({
                "commands": arguments.get("commands"),
            }))),
        };

        let (artifact_id, revision) = match artifact_kind {
            WorkflowArtifactKind::DirectCommands => {
                let existing = previous_artifact
                    .as_ref()
                    .filter(|artifact| artifact.kind == artifact_kind);
                let changed = existing
                    .zip(candidate_digest.as_ref())
                    .is_none_or(|(artifact, digest)| artifact.digest != *digest);
                (
                    Some(
                        existing
                            .map(|artifact| artifact.artifact_id.clone())
                            .unwrap_or_else(|| "direct-commands".to_string()),
                    ),
                    Some(match existing {
                        Some(artifact) if changed => artifact.revision.saturating_add(1),
                        Some(artifact) => artifact.revision,
                        None => 1,
                    }),
                )
            }
            WorkflowArtifactKind::FlowScript if explicit_draft_id.is_none() && source.is_some() => {
                let existing = previous_artifact
                    .as_ref()
                    .filter(|artifact| artifact.kind == artifact_kind);
                let changed = existing
                    .zip(candidate_digest.as_ref())
                    .is_none_or(|(artifact, digest)| artifact.digest != *digest);
                (
                    Some(
                        existing
                            .map(|artifact| artifact.artifact_id.clone())
                            .unwrap_or_else(|| "flowscript-session".to_string()),
                    ),
                    Some(explicit_revision.unwrap_or_else(|| match existing {
                        Some(artifact) if changed => artifact.revision.saturating_add(1),
                        Some(artifact) => artifact.revision,
                        None => 1,
                    })),
                )
            }
            _ => (explicit_draft_id.map(str::to_string), explicit_revision),
        };

        let artifact_candidate = if rejected_identity {
            None
        } else {
            artifact_id
                .zip(revision)
                .zip(candidate_digest)
                .map(|((artifact_id, revision), digest)| (artifact_id, revision, digest))
        };
        let artifact_progressed =
            artifact_candidate
                .as_ref()
                .is_some_and(|(artifact_id, revision, digest)| {
                    previous_artifact.as_ref().is_none_or(|previous| {
                        previous.kind != artifact_kind
                            || previous.artifact_id != *artifact_id
                            || previous.revision != *revision
                            || previous.digest != *digest
                    })
                });

        let validation_status = match status {
            Some("valid" | "queued" | "already_queued") => Some(WorkflowValidationStatus::Valid),
            Some("validation_errors" | "invalid" | "error") => {
                Some(WorkflowValidationStatus::Invalid)
            }
            Some("interrupted" | "edit_interrupted") => Some(WorkflowValidationStatus::Interrupted),
            _ if artifact_kind == WorkflowArtifactKind::DirectCommands => {
                Some(WorkflowValidationStatus::Valid)
            }
            _ => None,
        };
        let diagnostic_fingerprint = parsed.as_ref().and_then(|value| {
            let material = json!({
                "diagnostics": value.get("diagnostics"),
                "errors": value.get("errors"),
                "missing_modules": value.get("missing_modules"),
                "code": value.get("code"),
            });
            material
                .as_object()
                .is_some_and(|object| object.values().any(|value| !value.is_null()))
                .then(|| workflow_strategy_fingerprint(&material))
        });
        let validation_progressed =
            validation_status
                .zip(revision)
                .is_some_and(|(status, revision)| {
                    self.validation.as_ref().is_none_or(|previous| {
                        previous.status != status
                            || previous.artifact_revision != revision
                            || previous.diagnostic_fingerprint != diagnostic_fingerprint
                    })
                });
        let should_prepare = matches!(status, Some("queued" | "already_queued"))
            || artifact_kind == WorkflowArtifactKind::DirectCommands;
        let prepare_progressed = should_prepare
            && revision.is_some_and(|revision| {
                self.prepared
                    .as_ref()
                    .is_none_or(|prepared| prepared.artifact_revision != revision)
            });

        let mut observation = WorkflowToolObservation::default();
        if let Some((artifact_id, revision, digest)) = artifact_candidate {
            self.record_artifact(artifact_kind, artifact_id, revision, digest, elapsed_ms)?;
            observation.artifact_retained = true;
        }
        if let (Some(status), Some(revision)) = (validation_status, revision)
            && self
                .artifact
                .as_ref()
                .is_some_and(|artifact| artifact.revision == revision)
        {
            self.record_validation(status, revision, diagnostic_fingerprint.clone(), elapsed_ms)?;
            observation.validation_recorded = true;
        }
        if should_prepare
            && let Some(artifact) = self.artifact.as_ref()
            && self.validation.as_ref().is_some_and(|validation| {
                validation.status == WorkflowValidationStatus::Valid
                    && validation.artifact_revision == artifact.revision
            })
        {
            let review_id = format!("{}:{}", artifact.artifact_id, artifact.revision);
            let artifact_revision = artifact.revision;
            self.prepare_review(review_id, artifact_revision, elapsed_ms)?;
            observation.review_prepared = true;
        }

        let lifecycle_progressed =
            artifact_progressed || validation_progressed || prepare_progressed;
        let progress_fingerprint = workflow_strategy_fingerprint(&json!({
            "artifact": self.artifact,
            "validation": self.validation,
            "prepared": self.prepared,
        }));
        let strategy = json!({ "tool": tool_name, "arguments": arguments });
        observation.strategy_decision = Some(if lifecycle_progressed {
            self.note_progress(progress_fingerprint);
            let strategy_fingerprint = workflow_strategy_fingerprint(&strategy);
            self.telemetry.append(
                elapsed_ms,
                WorkflowTelemetryKind::StrategyAttempted,
                json!({
                    "strategy_fingerprint": strategy_fingerprint,
                    "progressed": true,
                }),
            );
            StrategyDecision::Progressed {
                strategy_fingerprint,
            }
        } else {
            self.record_strategy_attempt(&strategy, None, elapsed_ms)?
        });
        Ok(observation)
    }

    pub fn prepare_review(
        &mut self,
        review_id: impl Into<String>,
        artifact_revision: u64,
        elapsed_ms: u64,
    ) -> Result<(), WorkflowSessionError> {
        self.require_active("prepare review")?;
        let artifact = self
            .artifact
            .as_ref()
            .ok_or(WorkflowSessionError::ArtifactRequired("prepare review"))?;
        if artifact.revision != artifact_revision {
            return Err(WorkflowSessionError::RevisionMismatch {
                expected: artifact.revision,
                received: artifact_revision,
            });
        }
        if self.validation.as_ref().is_none_or(|validation| {
            validation.status != WorkflowValidationStatus::Valid
                || validation.artifact_revision != artifact_revision
        }) {
            return Err(WorkflowSessionError::ValidArtifactRequired);
        }
        let review_id = review_id.into();
        self.prepared = Some(PreparedWorkflowState {
            review_id: review_id.clone(),
            artifact_revision,
            prepared_at_ms: elapsed_ms,
        });
        self.phase = WorkflowSessionPhase::Prepared;
        self.telemetry.append(
            elapsed_ms,
            WorkflowTelemetryKind::ReviewPrepared,
            json!({
                "review_id": review_id,
                "artifact_revision": artifact_revision,
            }),
        );
        Ok(())
    }

    pub fn request_approval(&mut self, elapsed_ms: u64) -> Result<(), WorkflowSessionError> {
        self.require_phase(WorkflowSessionPhase::Prepared, "request approval")?;
        self.phase = WorkflowSessionPhase::AwaitingApproval;
        self.telemetry.append(
            elapsed_ms,
            WorkflowTelemetryKind::ApprovalRequested,
            json!({
                "review_id": self.prepared.as_ref().map(|prepared| &prepared.review_id),
            }),
        );
        Ok(())
    }

    pub fn begin_apply(&mut self, elapsed_ms: u64) -> Result<(), WorkflowSessionError> {
        self.require_phase(WorkflowSessionPhase::AwaitingApproval, "begin apply")?;
        self.phase = WorkflowSessionPhase::Applying;
        self.telemetry.append(
            elapsed_ms,
            WorkflowTelemetryKind::ApplyStarted,
            json!({
                "review_id": self.prepared.as_ref().map(|prepared| &prepared.review_id),
            }),
        );
        Ok(())
    }

    pub fn mark_applied(&mut self, elapsed_ms: u64) -> Result<(), WorkflowSessionError> {
        self.require_phase(WorkflowSessionPhase::Applying, "mark applied")?;
        self.phase = WorkflowSessionPhase::Applied;
        self.telemetry.append(
            elapsed_ms,
            WorkflowTelemetryKind::Applied,
            json!({
                "review_id": self.prepared.as_ref().map(|prepared| &prepared.review_id),
            }),
        );
        Ok(())
    }

    pub fn dismiss(
        &mut self,
        reason: impl Into<String>,
        elapsed_ms: u64,
    ) -> Result<(), WorkflowSessionError> {
        self.require_active("dismiss review")?;
        if !matches!(
            self.phase,
            WorkflowSessionPhase::Prepared | WorkflowSessionPhase::AwaitingApproval
        ) {
            return Err(WorkflowSessionError::InvalidTransition {
                from: self.phase,
                operation: "dismiss review",
            });
        }
        let reason = reason.into();
        self.phase = WorkflowSessionPhase::Dismissed;
        self.terminal_reason = Some(reason.clone());
        self.telemetry.append(
            elapsed_ms,
            WorkflowTelemetryKind::Dismissed,
            json!({"reason": reason}),
        );
        Ok(())
    }

    pub fn fail(
        &mut self,
        reason: impl Into<String>,
        elapsed_ms: u64,
    ) -> Result<(), WorkflowSessionError> {
        self.require_active("fail session")?;
        let reason = reason.into();
        self.phase = WorkflowSessionPhase::Failed;
        self.terminal_reason = Some(reason.clone());
        self.telemetry.append(
            elapsed_ms,
            WorkflowTelemetryKind::Failed,
            json!({"reason": reason}),
        );
        Ok(())
    }

    pub fn cancel(
        &mut self,
        reason: impl Into<String>,
        elapsed_ms: u64,
    ) -> Result<(), WorkflowSessionError> {
        self.require_active("cancel session")?;
        let reason = reason.into();
        self.phase = WorkflowSessionPhase::Cancelled;
        self.terminal_reason = Some(reason.clone());
        self.telemetry.append(
            elapsed_ms,
            WorkflowTelemetryKind::Cancelled,
            json!({"reason": reason}),
        );
        Ok(())
    }

    /// Snapshot is a pure projection: it reads no clock and appends no telemetry. Supplying the
    /// same manifest, observations, and elapsed value always produces identical serialized bytes.
    pub fn snapshot(&self, elapsed_ms: u64) -> WorkflowSessionSnapshot {
        WorkflowSessionSnapshot {
            schema: WORKFLOW_SESSION_SNAPSHOT_VERSION.to_string(),
            manifest_fingerprint: self.manifest.fingerprint.clone(),
            policy: self.policy.clone(),
            phase: self.phase,
            elapsed_ms,
            first_artifact_sla: self.first_artifact_sla_status(elapsed_ms),
            artifact: self.artifact.clone(),
            validation: self.validation.clone(),
            prepared: self.prepared.clone(),
            context_reads: self.context_reads.iter().cloned().collect(),
            in_flight_context_reads: self.in_flight_context_reads.keys().cloned().collect(),
            predraft_unique_context_reads: self.predraft_unique_context_reads,
            strategy_attempts: self.strategy_attempts.clone(),
            consecutive_zero_progress_attempts: self.consecutive_zero_progress_attempts,
            circuit: self.circuit.clone(),
            terminal_reason: self.terminal_reason.clone(),
            telemetry: self.telemetry.clone(),
        }
    }

    fn note_progress(&mut self, fingerprint: String) {
        self.last_progress_fingerprint = Some(fingerprint);
        self.consecutive_zero_progress_attempts = 0;
        self.circuit = None;
    }

    fn require_active(&self, operation: &'static str) -> Result<(), WorkflowSessionError> {
        if self.phase.is_terminal() {
            Err(WorkflowSessionError::Terminal {
                phase: self.phase,
                operation,
            })
        } else {
            Ok(())
        }
    }

    fn require_phase(
        &self,
        phase: WorkflowSessionPhase,
        operation: &'static str,
    ) -> Result<(), WorkflowSessionError> {
        self.require_active(operation)?;
        if self.phase == phase {
            Ok(())
        } else {
            Err(WorkflowSessionError::InvalidTransition {
                from: self.phase,
                operation,
            })
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WorkflowSessionError {
    #[error("cannot {operation}: workflow session is terminal in phase {phase:?}")]
    Terminal {
        phase: WorkflowSessionPhase,
        operation: &'static str,
    },
    #[error("cannot {operation} from workflow phase {from:?}")]
    InvalidTransition {
        from: WorkflowSessionPhase,
        operation: &'static str,
    },
    #[error("an artifact is required to {0}")]
    ArtifactRequired(&'static str),
    #[error("the current artifact must have a successful validation before review is prepared")]
    ValidArtifactRequired,
    #[error("artifact revision mismatch: expected {expected}, received {received}")]
    RevisionMismatch { expected: u64, received: u64 },
}

fn tool_context_domain(tool_name: &str) -> Option<ContextReadDomain> {
    match tool_name {
        "get_current_flowscript"
        | "get_node_details"
        | "get_unconfigured_nodes"
        | "list_board_nodes" => Some(ContextReadDomain::Board),
        "catalog_search" | "find_connectable_nodes" | "search_by_pin" => {
            Some(ContextReadDomain::Catalog)
        }
        "get_declarations" => Some(ContextReadDomain::Declarations),
        "database_tool" => Some(ContextReadDomain::Database),
        "ui_inspect" => Some(ContextReadDomain::Ui),
        "storage_tool" => Some(ContextReadDomain::Storage),
        _ => None,
    }
}

/// Interpret the semantic disposition inside provider-neutral tool text. Frontend bridges often
/// transport denied/time-out/cancelled JSON as a successful text envelope, so SDK metadata alone
/// cannot decide whether a reserved read should commit.
pub fn workflow_tool_result_succeeded(result_text: &str) -> bool {
    let trimmed = result_text.trim();
    if trimmed.to_ascii_lowercase().starts_with("error") {
        return false;
    }
    let Ok(payload) = serde_json::from_str::<Value>(trimmed) else {
        return true;
    };
    if payload
        .get("error")
        .is_some_and(|error| !error.is_null() && error.as_str() != Some(""))
        || payload
            .get("cancelled")
            .or_else(|| payload.get("canceled"))
            .and_then(Value::as_bool)
            == Some(true)
    {
        return false;
    }
    let status = payload
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    !matches!(
        status.as_str(),
        "error"
            | "failed"
            | "failure"
            | "cancelled"
            | "canceled"
            | "denied"
            | "timeout"
            | "timed_out"
            | "interrupted"
            | "stale"
            | "internal_state_unavailable"
            | "scope_violation"
    )
}

fn tool_result_proves_retained_artifact(tool_name: &str, result_text: &str) -> bool {
    if !matches!(
        tool_name,
        "edit_flowscript"
            | "write_flowscript"
            | "patch_flowscript"
            | "check_flowscript"
            | "commit_flowscript"
            | "begin_flow_ir_draft"
            | "update_flow_ir_draft"
            | "upsert_flow_ir_module"
            | "validate_flow_ir_draft"
            | "commit_flow_ir_draft"
    ) {
        return false;
    }
    let Some(payload) = parse_tool_result_value(result_text) else {
        return false;
    };
    payload
        .get("draft_id")
        .and_then(Value::as_str)
        .is_some_and(|draft_id| !draft_id.trim().is_empty())
        && payload.get("revision").and_then(Value::as_u64).is_some()
}

fn context_read_operation<'a>(tool_name: &'a str, arguments: &'a Value) -> &'a str {
    arguments
        .get("operation")
        .and_then(Value::as_str)
        .unwrap_or(tool_name)
}

/// A manifest slot covers only the inventory operation it actually performed. Completeness is
/// deliberately not domain-wide: schema collection does not contain table rows, a root file list
/// does not contain file contents, and a UI list does not contain page/widget detail payloads.
fn manifest_covers_context_read(
    manifest: &BoardContextManifest,
    tool_name: &str,
    arguments: &Value,
) -> bool {
    let operation = context_read_operation(tool_name, arguments);
    let slot = match tool_name {
        "database_tool" => manifest.augmentations.database.as_ref(),
        "ui_inspect" => manifest.augmentations.ui.as_ref(),
        "storage_tool" => manifest.augmentations.storage.as_ref(),
        _ => None,
    };
    let Some(payload) = slot.map(|slot| &slot.payload) else {
        return false;
    };
    if payload.get("complete").and_then(Value::as_bool) != Some(true) {
        return false;
    }

    match (tool_name, operation) {
        ("database_tool", "list_tables") => true,
        ("database_tool", "describe_table") => {
            if arguments
                .get("include_sample")
                .or_else(|| arguments.get("includeSample"))
                .and_then(Value::as_bool)
                != Some(false)
            {
                return false;
            }
            let requested = arguments
                .get("table_name")
                .or_else(|| arguments.get("tableName"))
                .and_then(Value::as_str);
            let user_scoped = arguments
                .get("user_scoped")
                .or_else(|| arguments.get("userScoped"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            requested.is_some_and(|requested| {
                payload
                    .get("tables")
                    .and_then(Value::as_array)
                    .is_some_and(|tables| {
                        tables.iter().any(|table| {
                            table.get("table_name").and_then(Value::as_str) == Some(requested)
                                && table.get("user_scoped").and_then(Value::as_bool)
                                    == Some(user_scoped)
                                && table.get("error").is_none()
                                && table.get("schema").is_some()
                        })
                    })
            })
        }
        ("ui_inspect", "list") => true,
        ("storage_tool", "list_files") => arguments
            .get("prefix")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .is_empty(),
        _ => false,
    }
}

fn source_argument(arguments: &Value) -> Option<&str> {
    ["flowscript", "script", "source", "content"]
        .into_iter()
        .find_map(|key| arguments.get(key).and_then(Value::as_str))
}

fn parse_tool_result_value(result_text: &str) -> Option<Value> {
    if let Ok(value) = serde_json::from_str(result_text.trim()) {
        return Some(value);
    }
    let extract = |tag: &str| -> Option<Value> {
        let start_marker = format!("<{tag}>");
        let end_marker = format!("</{tag}>");
        let start = result_text.find(&start_marker)? + start_marker.len();
        let end = result_text[start..].find(&end_marker)? + start;
        serde_json::from_str(&result_text[start..end]).ok()
    };
    // The draft/commit envelopes carry the lifecycle fields (draft_id, revision, status) that
    // progress accounting needs; the workspace/validation tags carry the retained source. Merge
    // them, envelope fields winning, so a write_flowscript result registers its revision instead
    // of scoring as a zero-progress attempt.
    let envelope =
        extract("flowscript_draft_result").or_else(|| extract("flowscript_commit_result"));
    let workspace = extract("flowscript_workspace").or_else(|| extract("validation"));
    match (envelope, workspace) {
        (Some(Value::Object(mut envelope)), Some(Value::Object(workspace))) => {
            for (key, value) in workspace {
                envelope.entry(key).or_insert(value);
            }
            Some(Value::Object(envelope))
        }
        (envelope @ Some(_), _) => envelope,
        (None, workspace) => workspace,
    }
}

fn canonicalize_json(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(canonicalize_json)
                .collect::<Vec<_>>(),
        ),
        Value::Object(values) => {
            let sorted = values
                .into_iter()
                .map(|(key, value)| (key, canonicalize_json(value)))
                .collect::<BTreeMap<_, _>>();
            Value::Object(sorted.into_iter().collect())
        }
        value => value,
    }
}

fn normalize_words(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn domain_hash(domain: &[u8], bytes: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    hasher.update(bytes);
    format!("b3:{}", hasher.finalize().to_hex())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::*;
    use crate::flow::copilot::{
        GraphContext, ManifestAudit, ManifestAugmentation, ManifestAugmentations, ManifestBoard,
        ManifestSource, default_flowscript_module_templates,
    };

    fn manifest() -> BoardContextManifest {
        BoardContextManifest::build(
            ManifestBoard {
                id: "board-1".to_string(),
                name: "Board".to_string(),
                description: "Session test".to_string(),
                version: (1, 0, 0),
                stage: "dev".to_string(),
                execution_mode: "hybrid".to_string(),
                refs: BTreeMap::new(),
                page_ids: vec![],
                graph: GraphContext {
                    nodes: vec![],
                    edges: vec![],
                    layers: vec![],
                    variables: vec![],
                    selected_nodes: vec![],
                },
            },
            &[],
            ManifestSource::absent(),
            ManifestAudit {
                request_identity: "request-1".to_string(),
                base_fingerprint: "base-1".to_string(),
                ..ManifestAudit::default()
            },
            ManifestAugmentations::default(),
            default_flowscript_module_templates(),
        )
        .expect("manifest")
    }

    fn policy() -> WorkflowSessionPolicy {
        WorkflowSessionPolicy {
            first_artifact_sla_ms: 100,
            max_predraft_context_reads: 2,
            max_zero_progress_attempts: 2,
            max_same_strategy_attempts: 2,
            telemetry_event_capacity: 4,
        }
    }

    #[test]
    fn context_reads_are_deduplicated_and_predraft_budget_is_shared() {
        let mut session = WorkflowSession::new(manifest(), policy());
        session.mark_manifest_ready(1).unwrap();
        session.begin_discovery(2).unwrap();

        let first = session
            .record_context_read(
                ContextReadDomain::Database,
                "List Tables",
                &json!({"schema": "public"}),
                3,
            )
            .unwrap();
        assert!(matches!(first, ContextReadDecision::Accepted { .. }));
        let duplicate = session
            .record_context_read(
                ContextReadDomain::Database,
                "  list   tables ",
                &json!({"schema": "public"}),
                4,
            )
            .unwrap();
        assert!(matches!(duplicate, ContextReadDecision::Duplicate { .. }));
        session
            .record_context_read(ContextReadDomain::Ui, "list pages", &json!({}), 5)
            .unwrap();
        let exhausted = session
            .record_context_read(ContextReadDomain::Storage, "list buckets", &json!({}), 6)
            .unwrap();
        assert!(matches!(
            exhausted,
            ContextReadDecision::PredraftBudgetExhausted { limit: 2, .. }
        ));

        session
            .record_artifact(
                WorkflowArtifactKind::FlowScript,
                "draft-1",
                1,
                "source-1",
                7,
            )
            .unwrap();
        let after_artifact = session
            .record_context_read(ContextReadDomain::Storage, "list buckets", &json!({}), 8)
            .unwrap();
        assert!(matches!(
            after_artifact,
            ContextReadDecision::Accepted { .. }
        ));
        assert_eq!(session.snapshot(8).predraft_unique_context_reads, 2);
    }

    #[test]
    fn context_preflight_commits_success_and_releases_failed_read_reservations() {
        let mut session = WorkflowSession::new(manifest(), policy());
        session.mark_manifest_ready(0).unwrap();
        session.begin_discovery(0).unwrap();
        let args = json!({"operation": "query", "table_name": "items"});

        let first = session
            .preflight_tool_call("database_tool", &args, 1)
            .unwrap();
        assert!(matches!(
            first,
            WorkflowToolPreflightDecision::Dispatch { lease: Some(_) }
        ));
        let first_lease = first.lease().cloned().unwrap();
        assert_eq!(session.snapshot(1).in_flight_context_reads.len(), 1);
        session
            .abort_tool_call(Some(&first_lease), "database_tool", &args, 2)
            .unwrap();
        assert!(session.snapshot(2).in_flight_context_reads.is_empty());
        assert_eq!(session.snapshot(2).predraft_unique_context_reads, 0);

        let retry = session
            .preflight_tool_call("database_tool", &args, 3)
            .unwrap();
        assert!(matches!(
            retry,
            WorkflowToolPreflightDecision::Dispatch { lease: Some(_) }
        ));
        let retry_lease = retry.lease().cloned().unwrap();
        session
            .complete_tool_call(Some(&first_lease), "database_tool", &args, "{}", true, 4)
            .unwrap();
        assert_eq!(session.snapshot(4).in_flight_context_reads.len(), 1);
        assert_eq!(session.snapshot(4).predraft_unique_context_reads, 0);
        session
            .complete_tool_call(Some(&retry_lease), "database_tool", &args, "{}", true, 5)
            .unwrap();
        assert_eq!(session.snapshot(5).predraft_unique_context_reads, 1);
        assert!(matches!(
            session.preflight_tool_call("database_tool", &args, 6),
            Ok(WorkflowToolPreflightDecision::ShortCircuit { ref code, .. })
                if code == "DUPLICATE_CONTEXT_READ"
        ));
    }

    #[test]
    fn semantic_tool_failures_do_not_commit_leases_or_infer_artifacts_from_arguments() {
        for result in [
            r#"{"status":"denied"}"#,
            r#"{"status":"timeout"}"#,
            r#"{"status":"scope_violation"}"#,
            r#"{"status":"ok","error":"frontend bridge failed"}"#,
            r#"{"cancelled":true}"#,
        ] {
            assert!(!workflow_tool_result_succeeded(result), "{result}");
        }
        assert!(workflow_tool_result_succeeded(r#"{"status":"ok"}"#));

        let mut session = WorkflowSession::new(manifest(), policy());
        session.mark_manifest_ready(0).unwrap();
        session.begin_discovery(0).unwrap();
        let args = json!({
            "draft_id": "model-guessed-draft",
            "source": "eventsSimple() {}"
        });
        session
            .complete_tool_call(
                None,
                "write_flowscript",
                &args,
                r#"{"status":"error","message":"worker failed before retention"}"#,
                false,
                1,
            )
            .unwrap();
        assert!(session.snapshot(1).artifact.is_none());

        session
            .complete_tool_call(
                None,
                "write_flowscript",
                &args,
                r#"{"status":"validation_errors","draft_id":"host-draft","revision":0,"source":"eventsSimple() {}"}"#,
                false,
                2,
            )
            .unwrap();
        assert_eq!(
            session
                .snapshot(2)
                .artifact
                .as_ref()
                .map(|artifact| artifact.artifact_id.as_str()),
            Some("host-draft")
        );
    }

    #[test]
    fn manifest_completeness_is_operation_specific_not_domain_wide() {
        let mut manifest = manifest();
        manifest.augmentations.database = Some(ManifestAugmentation::new(
            "test#database",
            None,
            json!({
                "complete": true,
                "tables": [{
                    "table_name": "items",
                    "user_scoped": false,
                    "schema": {"fields": [{"name": "id"}]},
                    "indices": []
                }]
            }),
        ));
        manifest.augmentations.storage = Some(ManifestAugmentation::new(
            "test#storage",
            None,
            json!({"complete": true, "project_items": [], "user_items": []}),
        ));
        manifest.augmentations.ui = Some(ManifestAugmentation::new(
            "test#ui",
            None,
            json!({"complete": true, "pages": [], "widgets": []}),
        ));
        let mut session = WorkflowSession::new(manifest, policy());

        for (tool, args) in [
            ("database_tool", json!({"operation": "list_tables"})),
            (
                "database_tool",
                json!({"operation": "describe_table", "table_name": "items", "user_scoped": false, "include_sample": false}),
            ),
            ("ui_inspect", json!({"operation": "list"})),
            (
                "storage_tool",
                json!({"operation": "list_files", "prefix": ""}),
            ),
        ] {
            assert!(matches!(
                session.preflight_tool_call(tool, &args, 1),
                Ok(WorkflowToolPreflightDecision::ShortCircuit { ref code, .. })
                    if code == "CONTEXT_ALREADY_IN_MANIFEST"
            ));
        }

        let sample_read = json!({
            "operation": "describe_table",
            "table_name": "items",
            "user_scoped": false,
            "include_sample": true
        });
        let sample_decision = session
            .preflight_tool_call("database_tool", &sample_read, 2)
            .unwrap();
        assert!(matches!(
            sample_decision,
            WorkflowToolPreflightDecision::Dispatch { lease: Some(_) }
        ));
        session
            .abort_tool_call(sample_decision.lease(), "database_tool", &sample_read, 3)
            .unwrap();

        for (tool, args) in [
            (
                "database_tool",
                json!({"operation": "query", "table_name": "items"}),
            ),
            (
                "storage_tool",
                json!({"operation": "read_file", "path": "config.json"}),
            ),
        ] {
            let decision = session.preflight_tool_call(tool, &args, 2).unwrap();
            assert!(matches!(
                decision,
                WorkflowToolPreflightDecision::Dispatch { lease: Some(_) }
            ));
            session
                .abort_tool_call(decision.lease(), tool, &args, 3)
                .unwrap();
        }
    }

    #[test]
    fn first_artifact_sla_is_observed_once_and_satisfied_by_retention() {
        let mut session = WorkflowSession::new(manifest(), policy());
        assert_eq!(
            session.observe_first_artifact_sla(40),
            FirstArtifactSlaStatus::Pending { remaining_ms: 60 }
        );
        assert_eq!(
            session.observe_first_artifact_sla(125),
            FirstArtifactSlaStatus::Breached { overdue_ms: 25 }
        );
        session.observe_first_artifact_sla(150);
        assert_eq!(
            session
                .telemetry()
                .count(WorkflowTelemetryKind::FirstArtifactSlaBreached),
            1
        );
        session
            .record_artifact(
                WorkflowArtifactKind::FlowScript,
                "draft-1",
                1,
                "source-1",
                160,
            )
            .unwrap();
        assert_eq!(
            session.first_artifact_sla_status(200),
            FirstArtifactSlaStatus::Satisfied {
                first_retained_at_ms: 160
            }
        );
    }

    #[test]
    fn repeated_zero_progress_opens_circuit_and_new_progress_reopens_it() {
        let mut session = WorkflowSession::new(manifest(), policy());
        let strategy = json!({"repair": "replace missing call", "queries": ["smtp send"]});
        assert!(matches!(
            session
                .record_strategy_attempt(&strategy, None, 10)
                .unwrap(),
            StrategyDecision::RetryWithDifferentStrategy {
                attempts_remaining: 1,
                ..
            }
        ));
        let decision = session
            .record_strategy_attempt(&strategy, None, 20)
            .unwrap();
        assert!(matches!(
            decision,
            StrategyDecision::CircuitOpen {
                state: WorkflowCircuitState {
                    reason: CircuitOpenReason::RepeatedStrategy,
                    ..
                }
            }
        ));

        assert!(matches!(
            session
                .record_strategy_attempt(
                    &json!({"repair": "use alternate node"}),
                    Some("source-2"),
                    30
                )
                .unwrap(),
            StrategyDecision::Progressed { .. }
        ));
        assert!(session.snapshot(30).circuit.is_none());
        assert_eq!(session.snapshot(30).consecutive_zero_progress_attempts, 0);
    }

    #[test]
    fn valid_artifact_is_required_before_prepare_and_apply_lifecycle_is_explicit() {
        let mut session = WorkflowSession::new(manifest(), policy());
        session
            .record_artifact(
                WorkflowArtifactKind::FlowScript,
                "draft-1",
                2,
                "source-2",
                10,
            )
            .unwrap();
        assert_eq!(
            session.prepare_review("review-1", 2, 20),
            Err(WorkflowSessionError::ValidArtifactRequired)
        );
        session
            .record_validation(WorkflowValidationStatus::Valid, 2, None, 25)
            .unwrap();
        session.prepare_review("review-1", 2, 30).unwrap();
        session.request_approval(35).unwrap();
        session.begin_apply(40).unwrap();
        session.mark_applied(45).unwrap();
        assert_eq!(session.phase(), WorkflowSessionPhase::Applied);
        assert!(matches!(
            session.cancel("too late", 50),
            Err(WorkflowSessionError::Terminal { .. })
        ));
    }

    #[test]
    fn telemetry_compaction_is_bounded_without_silent_event_loss() {
        let mut ledger = WorkflowTelemetryLedger::new(2);
        ledger.append(1, WorkflowTelemetryKind::ContextRead, json!({"read": 1}));
        let digest_after_first = ledger.chain_digest().to_string();
        ledger.append(2, WorkflowTelemetryKind::ContextRead, json!({"read": 2}));
        ledger.append(3, WorkflowTelemetryKind::Applied, json!({"review": "r"}));

        assert_eq!(ledger.total_events(), 3);
        assert_eq!(ledger.compacted_events(), 1);
        assert_eq!(ledger.recent_events().len(), 2);
        assert_eq!(ledger.count(WorkflowTelemetryKind::ContextRead), 2);
        assert_ne!(ledger.chain_digest(), digest_after_first);
        let context_milestone = ledger
            .milestones()
            .get(&WorkflowTelemetryKind::ContextRead)
            .unwrap();
        assert_eq!(context_milestone.first.sequence, 0);
        assert_eq!(context_milestone.last.sequence, 1);
    }

    #[test]
    fn snapshots_are_byte_deterministic_for_the_same_observations() {
        let run = || {
            let mut session = WorkflowSession::new(manifest(), policy());
            session.mark_manifest_ready(1).unwrap();
            session.begin_discovery(2).unwrap();
            session
                .record_context_read(
                    ContextReadDomain::Database,
                    "list tables",
                    &json!({"z": 2, "a": 1}),
                    3,
                )
                .unwrap();
            session
                .record_artifact(
                    WorkflowArtifactKind::FlowScript,
                    "draft-1",
                    1,
                    "source-1",
                    4,
                )
                .unwrap();
            serde_json::to_vec(&session.snapshot(5)).unwrap()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn structured_strategy_fingerprint_is_object_order_independent() {
        assert_eq!(
            workflow_strategy_fingerprint(&json!({"b": 2, "a": 1})),
            workflow_strategy_fingerprint(&json!({"a": 1, "b": 2}))
        );
    }

    #[test]
    fn tool_observations_share_artifact_validation_review_and_circuit_semantics() {
        let mut session = WorkflowSession::new(manifest(), policy());
        session.mark_manifest_ready(0).unwrap();
        session.begin_discovery(0).unwrap();
        let args = json!({
            "draft_id": "draft-1",
            "expected_revision": 1,
        });
        let invalid = json!({
            "status": "validation_errors",
            "draft_id": "draft-1",
            "revision": 1,
            "source": "eventsSimple() { missingCall() }",
            "diagnostics": [{"code": "unknown-call", "message": "missingCall"}],
        })
        .to_string();

        let first = session
            .record_tool_result("check_flowscript", &args, &invalid, 10)
            .unwrap();
        assert!(first.artifact_retained);
        assert!(first.validation_recorded);
        assert!(matches!(
            first.strategy_decision,
            Some(StrategyDecision::Progressed { .. })
        ));

        let retry = session
            .record_tool_result("check_flowscript", &args, &invalid, 20)
            .unwrap();
        assert!(matches!(
            retry.strategy_decision,
            Some(StrategyDecision::RetryWithDifferentStrategy { .. })
        ));
        let stopped = session
            .record_tool_result("check_flowscript", &args, &invalid, 30)
            .unwrap();
        assert!(stopped.circuit_open());

        session.begin_continuation(40).unwrap();
        assert!(session.snapshot(40).circuit.is_none());
        let queued = json!({
            "status": "queued",
            "draft_id": "draft-1",
            "revision": 2,
            "source": "eventsSimple() { logInfo({ message: \"ready\" }) }",
            "diagnostics": [],
        })
        .to_string();
        let prepared = session
            .record_tool_result(
                "commit_flowscript",
                &json!({"draft_id": "draft-1", "expected_revision": 2}),
                &queued,
                50,
            )
            .unwrap();
        assert!(prepared.review_prepared);
        assert_eq!(session.phase(), WorkflowSessionPhase::Prepared);
    }

    #[test]
    fn direct_commands_use_the_same_prepared_artifact_lifecycle() {
        let mut session = WorkflowSession::new(manifest(), policy());
        let args = json!({
            "commands": [{
                "command_type": "MoveNode",
                "node_id": "node-1",
                "position": {"x": 10, "y": 20},
                "summary": "Align"
            }],
            "explanation": "Align the workflow"
        });
        let observed = session
            .record_tool_result(
                "emit_commands",
                &args,
                "<commands>[{\"command_type\":\"MoveNode\"}]</commands>",
                5,
            )
            .unwrap();

        assert!(observed.artifact_retained);
        assert!(observed.validation_recorded);
        assert!(observed.review_prepared);
        assert_eq!(
            session.snapshot(5).artifact.unwrap().kind,
            WorkflowArtifactKind::DirectCommands
        );
        assert_eq!(session.phase(), WorkflowSessionPhase::Prepared);
    }

    /// Real write/patch/check results ship the lifecycle fields in the
    /// `<flowscript_draft_result>` envelope beside the `<flowscript_workspace>` tag. Progress
    /// accounting must read that envelope: before it did, every write_flowscript result had
    /// revision=None, scored as zero progress, and two prompt-endorsed whole-document rewrites
    /// opened the circuit and terminated the run with no FlowScript.
    fn rendered_draft_result(status: &str, draft_id: &str, revision: u64, source: &str) -> String {
        format!(
            "<flowscript_workspace>{}</flowscript_workspace>\n<flowscript_draft_result>{}</flowscript_draft_result>",
            json!({ "source": source, "status": status }),
            json!({
                "status": status,
                "message": "retained",
                "draft_id": draft_id,
                "revision": revision,
                "source_bytes": source.len(),
            }),
        )
    }

    #[test]
    fn rendered_write_results_register_progress_and_only_true_noops_open_the_circuit() {
        let mut session = WorkflowSession::new(manifest(), policy());
        session.mark_manifest_ready(0).unwrap();
        session.begin_discovery(0).unwrap();

        let first_source = "eventsSimple() { logInfo({ message: \"draft\" }) }";
        let first = session
            .record_tool_result(
                "write_flowscript",
                &json!({ "draft_id": "draft-1", "source": first_source }),
                &rendered_draft_result("draft_started", "draft-1", 0, first_source),
                10,
            )
            .unwrap();
        assert!(first.artifact_retained);
        assert!(matches!(
            first.strategy_decision,
            Some(StrategyDecision::Progressed { .. })
        ));
        let artifact = session.snapshot(10).artifact.expect("artifact retained");
        assert_eq!(artifact.artifact_id, "draft-1");
        assert_eq!(artifact.revision, 0);

        // The prompt-endorsed whole-document repair: same draft id, replace_existing, new source.
        let second_source = "eventsSimple() { logInfo({ message: \"repaired draft\" }) }";
        let rewrite_args = json!({
            "draft_id": "draft-1",
            "replace_existing": true,
            "source": second_source,
        });
        let second = session
            .record_tool_result(
                "write_flowscript",
                &rewrite_args,
                &rendered_draft_result("draft_updated", "draft-1", 1, second_source),
                20,
            )
            .unwrap();
        assert!(matches!(
            second.strategy_decision,
            Some(StrategyDecision::Progressed { .. })
        ));
        assert!(session.snapshot(20).circuit.is_none());

        // A literally identical resubmission is a genuine no-op and must still trip the circuit.
        let noop_result = rendered_draft_result("draft_updated", "draft-1", 1, second_source);
        let first_noop = session
            .record_tool_result("write_flowscript", &rewrite_args, &noop_result, 30)
            .unwrap();
        assert!(matches!(
            first_noop.strategy_decision,
            Some(StrategyDecision::RetryWithDifferentStrategy { .. })
        ));
        let second_noop = session
            .record_tool_result("write_flowscript", &rewrite_args, &noop_result, 40)
            .unwrap();
        assert!(second_noop.circuit_open());
    }

    #[test]
    fn fresh_draft_rebinding_and_salvage_revisions_never_error() {
        let mut session = WorkflowSession::new(manifest(), policy());
        session.mark_manifest_ready(0).unwrap();
        session.begin_discovery(0).unwrap();
        session
            .record_artifact(
                WorkflowArtifactKind::FlowScript,
                "draft-a",
                3,
                "digest-a",
                1,
            )
            .unwrap();

        // The host's stale-draft recovery instructs a fresh draft_id; recording it must rebind,
        // not error (the callers coerced that error into a run-terminating circuit-open).
        let rebound_source = "eventsSimple() { logInfo({ message: \"fresh draft\" }) }";
        let rebound = session
            .record_tool_result(
                "write_flowscript",
                &json!({ "draft_id": "draft-b", "source": rebound_source }),
                &rendered_draft_result("draft_started", "draft-b", 0, rebound_source),
                2,
            )
            .unwrap();
        assert!(rebound.artifact_retained);
        let artifact = session.snapshot(2).artifact.expect("artifact rebound");
        assert_eq!(artifact.artifact_id, "draft-b");
        assert_eq!(artifact.revision, 0);

        // A salvage commit at an older checked revision of the same draft is legal at the store
        // level; the session keeps its newer state and records no progress instead of failing.
        session
            .record_artifact(
                WorkflowArtifactKind::FlowScript,
                "draft-b",
                2,
                "digest-b2",
                3,
            )
            .unwrap();
        session
            .record_artifact(
                WorkflowArtifactKind::FlowScript,
                "draft-b",
                1,
                "digest-b1",
                4,
            )
            .unwrap();
        assert_eq!(
            session
                .snapshot(4)
                .artifact
                .expect("artifact kept")
                .revision,
            2
        );
    }
}
