//! Deterministic end-to-end simulator for the FlowPilot code-first loop.
//!
//! These tests drive the REAL model-facing tool surface — `write_flowscript`,
//! `patch_flowscript`, `check_flowscript`, `commit_flowscript` on a
//! [`FlowIrDraftStore`] — against the full native catalog and a real board,
//! but with a scripted agent instead of an LLM. Every scenario is cheap and
//! fully deterministic; the only thing simulated is the agent's decision
//! sequence, never the tool semantics.
//!
//! Failure injection covered: unknown declarations, stale draft revisions,
//! external board mutations (base fingerprint conflicts), ambiguous patches,
//! oversized sources, partially satisfied acceptance contracts, and
//! concurrent access to one draft.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::thread;
use std::time::SystemTime;

use flow_like::flow::ast::{
    FlowScriptDiagnosticCode, RenderOptions, apply_board_commands_to_board,
    apply_flowscript_to_board, board_to_flowscript, reconcile_text_with_catalog,
};
use flow_like::flow::board::{Board, ExecutionMode, ExecutionStage};
use flow_like::flow::copilot::{
    BoardCommand, CheckFlowScriptArgs, CommitFlowScriptArgs, FlowIrAcceptanceBinding,
    FlowIrDraftMode, FlowIrDraftStore, FlowScriptDraftResponse, NodeMetadata, PatchFlowScriptArgs,
    WriteFlowScriptArgs, node_to_metadata,
};
use flow_like::flow::execution::LogLevel;
use flow_like::flow::node::{Node, NodeLogic};
use flow_like::flow::pin::ValueType;
use flow_like::flow::variable::{Variable, VariableType};
use flow_like::state::{FlowLikeConfig, FlowLikeState};
use flow_like::utils::http::HTTPClient;
use flow_like_catalog::CatalogBuilder;
use flow_like_storage::object_store::path::Path;

// ── Shared fixtures ────────────────────────────────────────────────────

struct CatalogFixture {
    logic: Vec<Arc<dyn NodeLogic>>,
    nodes: Vec<Node>,
    metadata: Vec<NodeMetadata>,
}

/// The full native catalog, built once per test binary. Scenarios reconcile
/// against the real node/pin contracts, not hand-written test metadata.
static FIXTURE: LazyLock<CatalogFixture> = LazyLock::new(|| {
    let logic = CatalogBuilder::new().build();
    let nodes: Vec<Node> = logic.iter().map(|logic| logic.get_node()).collect();
    let metadata: Vec<NodeMetadata> = nodes.iter().map(node_to_metadata).collect();
    CatalogFixture {
        logic,
        nodes,
        metadata,
    }
});

fn empty_board(id: &str) -> Board {
    let mut board = Board::new_detached(Some(id.to_string()), Path::default());
    board.name = format!("Simulator Board {id}");
    board.description.clear();
    board.viewport = (0.0, 0.0, 1.0);
    board.hash = None;
    board.created_at = SystemTime::UNIX_EPOCH;
    board.updated_at = SystemTime::UNIX_EPOCH;
    board
}

/// A state whose node registry carries the real catalog logic, so apply-side
/// `on_update` passes (dynamic pins, variable refs) run exactly as in the app.
async fn catalog_state() -> Arc<FlowLikeState> {
    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ));
    let registry = state.node_registry();
    registry.write().await.push_nodes(FIXTURE.logic.clone());
    state
}

/// Realistic multi-function support-mail-shaped program: helper functions with
/// a mutable local and a literal return, an approval-style branch in the event,
/// and a `string_format` call whose `{sender}` placeholder exercises dynamic
/// on_update pins end to end.
const GOLDEN_SCRIPT: &str = r#"function buildReply(subject: string): (reply: string) {
    let reply = stringTrim({ string: subject })
    if (stringContains({ string: subject, substring: "URGENT" }).contains) {
        reply = stringToUpper({ string: subject })
    }
    return reply
}

function approvalNotice(): (notice: string) {
    return "approved by the reviewer"
}

eventsSimple() {
    const summary = stringFormat({ formatString: "Support mail from {sender}", sender: "customer@example.com" })
    const replyResult = buildReply({ subject: summary.formattedString })
    const notice = approvalNotice()
    if (stringContains({ string: replyResult.reply, substring: "URGENT" }).contains) {
        logInfo({ message: notice.notice })
    } else {
        logInfo({ message: replyResult.reply })
    }
}
"#;

fn simple_log_script(first: &str, second: &str) -> String {
    format!(
        "eventsSimple() {{\n    logInfo({{ message: {first:?} }})\n    logInfo({{ message: {second:?} }})\n}}\n"
    )
}

// ── Scripted agent ─────────────────────────────────────────────────────

/// One declarative step of a scripted agent turn. Steps that omit a revision
/// use the agent's tracked head revision (updated from every tool response),
/// mirroring a well-behaved model; `*At` variants inject stale or explicit
/// revisions for failure scenarios.
enum Step {
    Write {
        source: String,
    },
    Patch {
        old: &'static str,
        new: &'static str,
    },
    PatchAt {
        revision: u64,
        old: &'static str,
        new: &'static str,
    },
    Check,
    Commit,
    CommitAt {
        revision: u64,
    },
    /// Switch to a fresh draft id (same request binding), e.g. after a base
    /// fingerprint conflict told the agent to restart from the live board.
    StartNewDraft {
        draft_id: &'static str,
    },
    /// Another user/session edits the board outside this draft session.
    MutateBoardExternally,
    ExpectStatus {
        status: &'static str,
        code: Option<&'static str>,
    },
}

struct ScriptedAgent<'a> {
    store: &'a FlowIrDraftStore,
    board: Board,
    catalog: &'a [NodeMetadata],
    binding: Option<FlowIrAcceptanceBinding>,
    draft_id: String,
    revision: u64,
    last: Option<FlowScriptDraftResponse>,
    external_markers: usize,
}

impl<'a> ScriptedAgent<'a> {
    fn new(
        store: &'a FlowIrDraftStore,
        board: Board,
        catalog: &'a [NodeMetadata],
        draft_id: &str,
    ) -> Self {
        Self {
            store,
            board,
            catalog,
            binding: None,
            draft_id: draft_id.to_string(),
            revision: 0,
            last: None,
            external_markers: 0,
        }
    }

    /// Bind the host-derived acceptance contract for the original user request.
    fn bind(&mut self, prompt: &str) {
        self.binding = Some(
            self.store
                .bind_request_acceptance_contract(&self.board.id, prompt),
        );
    }

    fn record(&mut self, response: FlowScriptDraftResponse) {
        if let Some(revision) = response.revision {
            self.revision = revision;
        }
        self.last = Some(response);
    }

    fn last(&self) -> &FlowScriptDraftResponse {
        self.last.as_ref().expect("a tool step ran before this")
    }

    fn write(&mut self, source: String) {
        let args = WriteFlowScriptArgs {
            draft_id: self.draft_id.clone(),
            replace_existing: false,
            mode: FlowIrDraftMode::Additive,
            source,
            allow_scope_reduction: false,
        };
        let response = match &self.binding {
            Some(binding) => self.store.write_flowscript_with_acceptance_binding(
                &self.board,
                self.catalog,
                args,
                binding,
            ),
            None => self.store.write_flowscript(&self.board, self.catalog, args),
        };
        self.record(response);
    }

    fn patch_at(&mut self, revision: u64, old: &str, new: &str) {
        let args = PatchFlowScriptArgs {
            draft_id: self.draft_id.clone(),
            expected_revision: revision,
            old_text: old.to_string(),
            new_text: new.to_string(),
            allow_scope_reduction: false,
        };
        let response = match &self.binding {
            Some(binding) => self.store.patch_flowscript_with_acceptance_binding(
                &self.board,
                self.catalog,
                args,
                binding,
            ),
            None => self.store.patch_flowscript(&self.board, self.catalog, args),
        };
        self.record(response);
    }

    fn check(&mut self) {
        let args = CheckFlowScriptArgs {
            draft_id: self.draft_id.clone(),
            expected_revision: self.revision,
        };
        let response = match &self.binding {
            Some(binding) => self.store.check_flowscript_with_acceptance_binding(
                &self.board,
                self.catalog,
                args,
                binding,
            ),
            None => self.store.check_flowscript(&self.board, self.catalog, args),
        };
        self.record(response);
    }

    fn commit_at(&mut self, revision: u64) {
        let args = CommitFlowScriptArgs {
            draft_id: self.draft_id.clone(),
            expected_revision: revision,
            allow_deletions: false,
            remove_node_ids: Vec::new(),
            remove_variable_ids: Vec::new(),
            remove_layer_ids: Vec::new(),
            remove_comment_ids: Vec::new(),
        };
        let response = match &self.binding {
            Some(binding) => self.store.commit_flowscript_with_acceptance_binding(
                &self.board,
                self.catalog,
                args,
                binding,
            ),
            None => self
                .store
                .commit_flowscript(&self.board, self.catalog, args),
        };
        self.record(response);
    }

    fn mutate_board_externally(&mut self) {
        self.external_markers += 1;
        let mut marker = Variable::new(
            &format!("externalMarker{}", self.external_markers),
            VariableType::String,
            ValueType::Normal,
        );
        marker.id = format!("external-marker-{}", self.external_markers);
        self.board.variables.insert(marker.id.clone(), marker);
    }

    fn expect_status(&self, status: &str, code: Option<&str>) {
        let last = self.last();
        assert_eq!(
            last.status, status,
            "expected status {status:?} (code {code:?}) but got: {last:#?}"
        );
        if let Some(code) = code {
            assert_eq!(
                last.code.as_deref(),
                Some(code),
                "expected code {code:?} but got: {last:#?}"
            );
        }
    }

    fn run(&mut self, steps: Vec<Step>) {
        for step in steps {
            match step {
                Step::Write { source } => self.write(source),
                Step::Patch { old, new } => self.patch_at(self.revision, old, new),
                Step::PatchAt { revision, old, new } => self.patch_at(revision, old, new),
                Step::Check => self.check(),
                Step::Commit => self.commit_at(self.revision),
                Step::CommitAt { revision } => self.commit_at(revision),
                Step::StartNewDraft { draft_id } => {
                    self.draft_id = draft_id.to_string();
                    self.revision = 0;
                }
                Step::MutateBoardExternally => self.mutate_board_externally(),
                Step::ExpectStatus { status, code } => self.expect_status(status, code),
            }
        }
    }
}

fn assert_noop_roundtrip(board: &Board, context: &str) {
    let anchored = board_to_flowscript(
        board,
        &RenderOptions {
            anchors: true,
            ..RenderOptions::default()
        },
    );
    let result = reconcile_text_with_catalog(board, &anchored, &FIXTURE.metadata);
    assert!(
        result.diagnostics.is_empty(),
        "{context}: re-reconciling the lowered board produced diagnostics:\n{:#?}\nsource:\n{anchored}",
        result.diagnostics
    );
    assert!(
        result.commands.is_empty(),
        "{context}: re-reconciling the lowered board produced {} command(s):\n{:#?}\nsource:\n{anchored}",
        result.commands.len(),
        result.commands
    );
}

// ── Scenarios ──────────────────────────────────────────────────────────

/// Happy path: write → check → commit a realistic multi-function program,
/// apply the exact queued batch to the board, and prove the applied board
/// lowers back to a document that reconciles to zero commands and zero
/// diagnostics against itself.
#[tokio::test]
async fn golden_path() {
    let fixture = &*FIXTURE;
    let store = FlowIrDraftStore::new();
    let mut agent = ScriptedAgent::new(
        &store,
        empty_board("sim-golden"),
        &fixture.metadata,
        "golden-draft",
    );
    agent.bind("Trim the incoming support mail subject, then log the reply and the reviewer approval notice.");
    agent.run(vec![
        Step::Write {
            source: GOLDEN_SCRIPT.to_string(),
        },
        Step::ExpectStatus {
            status: "draft_started",
            code: None,
        },
        Step::Check,
        Step::ExpectStatus {
            status: "valid",
            code: None,
        },
        Step::Commit,
        Step::ExpectStatus {
            status: "queued",
            code: None,
        },
    ]);
    let queued = agent.last();
    assert!(
        queued.diagnostics.is_empty(),
        "queued response must carry no blocking diagnostics: {queued:#?}"
    );
    let commands: Vec<BoardCommand> = queued.commands.clone();
    assert!(
        !commands.is_empty(),
        "the golden commit must queue board commands"
    );

    let state = catalog_state().await;
    let mut board = agent.board.clone();
    let applied = apply_board_commands_to_board(&mut board, commands, &fixture.nodes, state, None)
        .await
        .expect("queued golden batch applies");
    assert!(
        applied.diagnostics.is_empty(),
        "apply diagnostics: {:#?}",
        applied.diagnostics
    );
    assert!(!applied.commands.is_empty(), "apply executed commands");
    assert!(!board.nodes.is_empty(), "apply created nodes on the board");

    assert_noop_roundtrip(&board, "golden_path");
}

/// The uptime-monitor regression shape: a function whose declared return
/// values come from a multi-output catalog node (`utils_user_get_executing_user`)
/// through member-access chains. The write must check valid, the queued batch
/// must feed every declared boundary return pin, and the applied board must
/// lower back to a no-op — protecting multi-value return wiring end to end.
const MULTI_OUTPUT_RETURN_SCRIPT: &str = r#"function getOwnerIdentity(salt: string): (ownerSub: string, ownerKey: string, hasUser: bool, echoedSalt: string) {
    const user = utilsUserGetExecutingUser()
    const asText = valToString({ value: user.userContext.sub, pretty: false })
    const hashed = utilsHashSha256({ input: asText.string })
    return hashed.hash, asText.string, user.hasUser, salt
}

eventsSimple() {
    const identity = getOwnerIdentity({ salt: "seed" })
    if (identity.hasUser) {
        logInfo({ message: identity.ownerKey })
    } else {
        logInfo({ message: "no user" })
    }
}
"#;

#[tokio::test]
async fn multi_output_return_golden_path() {
    let fixture = &*FIXTURE;
    let store = FlowIrDraftStore::new();
    let mut agent = ScriptedAgent::new(
        &store,
        empty_board("sim-multi-return"),
        &fixture.metadata,
        "multi-return-draft",
    );
    agent.bind("Resolve the executing user's identity and log their owner key.");
    agent.run(vec![
        Step::Write {
            source: MULTI_OUTPUT_RETURN_SCRIPT.to_string(),
        },
        Step::ExpectStatus {
            status: "draft_started",
            code: None,
        },
        Step::Check,
        Step::ExpectStatus {
            status: "valid",
            code: None,
        },
        Step::Commit,
        Step::ExpectStatus {
            status: "queued",
            code: None,
        },
    ]);
    let queued = agent.last();
    assert!(
        queued.diagnostics.is_empty(),
        "queued response must carry no blocking diagnostics: {queued:#?}"
    );
    let commands: Vec<BoardCommand> = queued.commands.clone();

    let state = catalog_state().await;
    let mut board = agent.board.clone();
    let applied = apply_board_commands_to_board(&mut board, commands, &fixture.nodes, state, None)
        .await
        .expect("queued multi-output-return batch applies");
    assert!(
        applied.diagnostics.is_empty(),
        "apply diagnostics: {:#?}",
        applied.diagnostics
    );

    let layer = board
        .layers
        .values()
        .find(|layer| layer.name == "getOwnerIdentity")
        .expect("function layer exists on the applied board");
    // `echoedSalt` returns a bare function PARAMETER. That value both enters and leaves the same
    // layer, so it must route through a spliced `reroute` inside the layer — a direct boundary
    // self-edge is rejected by `connect_pins` and rolls the whole apply batch back.
    for return_pin in ["ownerSub", "ownerKey", "hasUser", "echoedSalt"] {
        let boundary = layer
            .pins
            .values()
            .find(|pin| pin.name == *return_pin)
            .unwrap_or_else(|| panic!("missing boundary return pin {return_pin}"));
        assert!(
            !boundary.depends_on.is_empty(),
            "declared return pin `{return_pin}` must be fed by the body"
        );
    }

    assert_noop_roundtrip(&board, "multi_output_return_golden_path");
}

/// Commit runs the required check inline: write → commit with no explicit check queues the
/// exact derived batch, saving the model round that used to bounce with
/// FLOWSCRIPT_CHECK_REQUIRED. An invalid source still returns validation_errors and queues
/// nothing.
#[test]
fn commit_runs_check_inline() {
    let fixture = &*FIXTURE;
    let store = FlowIrDraftStore::new();
    let mut agent = ScriptedAgent::new(
        &store,
        empty_board("sim-inline-check"),
        &fixture.metadata,
        "inline-check-draft",
    );
    agent.run(vec![
        Step::Write {
            source: simple_log_script("triage the mail", "notify the reviewer"),
        },
        Step::ExpectStatus {
            status: "draft_started",
            code: None,
        },
        Step::Commit,
        Step::ExpectStatus {
            status: "queued",
            code: None,
        },
    ]);
    assert!(!agent.last().commands.is_empty());

    let mut invalid = ScriptedAgent::new(
        &store,
        empty_board("sim-inline-check-invalid"),
        &fixture.metadata,
        "inline-check-invalid",
    );
    invalid.run(vec![
        Step::Write {
            source: "eventsSimple() {\n    definitelyNotACatalogNode({ value: 1 })\n}\n"
                .to_string(),
        },
        Step::Commit,
        Step::ExpectStatus {
            status: "validation_errors",
            code: None,
        },
    ]);
    assert!(invalid.last().commands.is_empty());
}

/// A source with an unknown node name yields actionable structured
/// diagnostics; one unique text patch repairs it in place, after which the
/// same draft checks valid and commits.
#[test]
fn validation_repair_path() {
    let fixture = &*FIXTURE;
    let store = FlowIrDraftStore::new();
    let mut agent = ScriptedAgent::new(
        &store,
        empty_board("sim-repair"),
        &fixture.metadata,
        "repair-draft",
    );
    agent.run(vec![
        Step::Write {
            // `stringFormatter` is a near-miss of the real `string_format`
            // node, so the diagnostic must carry actionable repair context.
            source: "eventsSimple() {\n    const banner = stringFormatter({ formatString: \"triage the mail\" })\n    logInfo({ message: banner.formattedString })\n}\n"
                .to_string(),
        },
        Step::ExpectStatus {
            status: "validation_errors",
            code: None,
        },
    ]);
    let written = agent.last();
    assert!(
        !written.diagnostics.is_empty(),
        "unknown node must produce diagnostics: {written:#?}"
    );
    let unknown = written
        .diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic.code == FlowScriptDiagnosticCode::FsCatalogDeclarationNotFound
                && diagnostic.declaration.as_deref() == Some("stringFormatter")
        })
        .unwrap_or_else(|| panic!("diagnostic must name the unknown declaration: {written:#?}"));
    let fix = unknown
        .fix
        .as_ref()
        .unwrap_or_else(|| panic!("unknown declaration must carry a repair fix: {written:#?}"));
    assert!(
        fix.catalog_declarations
            .iter()
            .any(|declaration| declaration.contains("stringFormat(")),
        "repair context must offer the real catalog declaration: {written:#?}"
    );

    agent.run(vec![
        Step::Patch {
            old: "stringFormatter",
            new: "stringFormat",
        },
        Step::ExpectStatus {
            status: "draft_updated",
            code: None,
        },
        Step::Check,
        Step::ExpectStatus {
            status: "valid",
            code: None,
        },
        Step::Commit,
        Step::ExpectStatus {
            status: "queued",
            code: None,
        },
    ]);
    assert!(!agent.last().commands.is_empty());
}

/// A patch against a stale expected_revision is rejected with a revision
/// conflict that reports the current head; retrying at the reported revision
/// succeeds.
#[test]
fn revision_conflict() {
    let fixture = &*FIXTURE;
    let store = FlowIrDraftStore::new();
    let mut agent = ScriptedAgent::new(
        &store,
        empty_board("sim-revision"),
        &fixture.metadata,
        "revision-draft",
    );
    agent.run(vec![
        Step::Write {
            source: simple_log_script("triage the mail", "notify the reviewer"),
        },
        Step::ExpectStatus {
            status: "draft_started",
            code: None,
        },
        Step::Patch {
            old: "triage the mail",
            new: "triage the support mail",
        },
        Step::ExpectStatus {
            status: "draft_updated",
            code: None,
        },
        // Second patch replays the original revision 0 — stale by one edit.
        Step::PatchAt {
            revision: 0,
            old: "notify the reviewer",
            new: "escalate to the reviewer",
        },
        Step::ExpectStatus {
            status: "error",
            code: Some("FLOWSCRIPT_REVISION_CONFLICT"),
        },
    ]);
    assert_eq!(
        agent.last().revision,
        Some(1),
        "the conflict must report the current head revision: {:#?}",
        agent.last()
    );
    // The conflict response told the agent the real head; the tracked-head
    // retry of the same edit succeeds.
    agent.run(vec![
        Step::Patch {
            old: "notify the reviewer",
            new: "escalate to the reviewer",
        },
        Step::ExpectStatus {
            status: "draft_updated",
            code: None,
        },
        Step::Check,
        Step::ExpectStatus {
            status: "valid",
            code: None,
        },
        Step::Commit,
        Step::ExpectStatus {
            status: "queued",
            code: None,
        },
    ]);
    assert_eq!(agent.last().revision, Some(2));
}

/// An external board edit after a valid check invalidates the base
/// fingerprint: check and commit both refuse with a base conflict, and the
/// SAME request binding can start a fresh draft against the mutated board —
/// still carrying the original acceptance contract (proved by the review
/// notes that keep firing on the recovered draft).
#[test]
fn base_fingerprint_conflict_recovery() {
    let fixture = &*FIXTURE;
    let store = FlowIrDraftStore::new();
    let mut agent = ScriptedAgent::new(
        &store,
        empty_board("sim-base-conflict"),
        &fixture.metadata,
        "base-conflict-draft",
    );
    // Two-part request: the source only implements the formatting half, so
    // the bound contract surfaces non-blocking incomplete-scope review notes.
    agent.bind("Format the customer message, then send a Slack notification.");
    let partial_source = "eventsSimple() {\n    const formatted = stringFormat({ formatString: \"customer message\" })\n    logInfo({ message: formatted.formattedString })\n}\n"
        .to_string();
    agent.run(vec![
        Step::Write {
            source: partial_source.clone(),
        },
        Step::ExpectStatus {
            status: "draft_started",
            code: None,
        },
        Step::Check,
        Step::ExpectStatus {
            status: "valid",
            code: None,
        },
    ]);
    let first_notes = agent.last().review_notes.clone();
    assert!(
        first_notes
            .iter()
            .any(|note| { note.code == FlowScriptDiagnosticCode::FsRequestAcceptanceIncomplete }),
        "the partial source must surface incomplete-scope review notes: {:#?}",
        agent.last()
    );

    agent.run(vec![
        Step::MutateBoardExternally,
        Step::Check,
        Step::ExpectStatus {
            status: "error",
            code: Some("FLOWSCRIPT_BASE_REVISION_CONFLICT"),
        },
        Step::Commit,
        Step::ExpectStatus {
            status: "error",
            code: Some("FLOWSCRIPT_BASE_REVISION_CONFLICT"),
        },
        // Recovery: the same binding starts a fresh draft from the live board.
        Step::StartNewDraft {
            draft_id: "base-conflict-recovered",
        },
        Step::Write {
            source: partial_source,
        },
        Step::ExpectStatus {
            status: "draft_started",
            code: None,
        },
        Step::Check,
        Step::ExpectStatus {
            status: "valid",
            code: None,
        },
    ]);
    assert_eq!(agent.last().revision, Some(0), "fresh draft starts at 0");
    assert!(
        agent
            .last()
            .review_notes
            .iter()
            .any(|note| { note.code == FlowScriptDiagnosticCode::FsRequestAcceptanceIncomplete }),
        "the recovered draft must carry the same request contract: {:#?}",
        agent.last()
    );

    agent.run(vec![
        Step::Commit,
        Step::ExpectStatus {
            status: "queued",
            code: None,
        },
    ]);
    assert!(!agent.last().commands.is_empty());
}

/// A failed (ambiguous) patch does not move the head, and a later head-moving
/// patch keeps the last fully checked revision salvageable: an explicit commit
/// at the retained checked revision still releases that exact batch.
#[test]
fn salvage_commit() {
    let fixture = &*FIXTURE;
    let store = FlowIrDraftStore::new();
    let mut agent = ScriptedAgent::new(
        &store,
        empty_board("sim-salvage"),
        &fixture.metadata,
        "salvage-draft",
    );
    agent.run(vec![
        Step::Write {
            source: simple_log_script("first support step", "second support step"),
        },
        Step::ExpectStatus {
            status: "draft_started",
            code: None,
        },
        Step::Check,
        Step::ExpectStatus {
            status: "valid",
            code: None,
        },
        // `logInfo` occurs twice — the patch is ambiguous and must be
        // rejected without changing the revision.
        Step::Patch {
            old: "logInfo",
            new: "logWarning",
        },
        Step::ExpectStatus {
            status: "error",
            code: Some("FLOWSCRIPT_PATCH_NOT_UNIQUE"),
        },
    ]);
    assert_eq!(agent.last().revision, Some(0), "failed patch keeps head");

    agent.run(vec![
        // A successful head edit invalidates the head check but keeps
        // revision 0 salvageable.
        Step::Patch {
            old: "first support step",
            new: "first triage step",
        },
        Step::ExpectStatus {
            status: "draft_updated",
            code: None,
        },
        // Explicit commit at the retained checked revision 0 still succeeds.
        Step::CommitAt { revision: 0 },
        Step::ExpectStatus {
            status: "queued",
            code: None,
        },
    ]);
    let queued = agent.last();
    assert_eq!(
        queued.revision,
        Some(0),
        "salvage commit restores the checked revision: {queued:#?}"
    );
    assert!(
        queued.message.contains("restored"),
        "salvage commit reports the restored revision: {queued:#?}"
    );
    assert!(!queued.commands.is_empty());
    assert!(
        queued
            .source
            .as_deref()
            .is_some_and(|source| source.contains("first support step")),
        "the salvaged batch belongs to the checked source, not the moved head: {queued:#?}"
    );
}

/// A source beyond the byte budget is rejected cleanly: no draft is retained,
/// the store is not corrupted, and the next write with the same id works.
#[test]
fn oversized_draft() {
    let fixture = &*FIXTURE;
    let store = FlowIrDraftStore::new();
    let board = empty_board("sim-oversized");
    let mut agent = ScriptedAgent::new(&store, board, &fixture.metadata, "oversized-draft");
    // MAX_FLOWSCRIPT_SOURCE_BYTES is 1 MiB; pad well past it.
    let oversized = format!(
        "eventsSimple() {{\n    logInfo({{ message: \"pad\" }})\n}}\n// {}",
        "x".repeat(1_200_000)
    );
    agent.run(vec![
        Step::Write { source: oversized },
        Step::ExpectStatus {
            status: "error",
            code: Some("FLOWSCRIPT_SOURCE_SIZE_LIMIT_EXCEEDED"),
        },
    ]);
    assert!(agent.last().draft_id.is_none(), "no draft was retained");
    assert!(
        !store.has_editable_draft_for_board(&agent.board.id),
        "the failed write must leave no editable draft behind"
    );

    // The same draft id is still free and the store fully functional.
    agent.run(vec![
        Step::Write {
            source: simple_log_script("triage the mail", "notify the reviewer"),
        },
        Step::ExpectStatus {
            status: "draft_started",
            code: None,
        },
        Step::Check,
        Step::ExpectStatus {
            status: "valid",
            code: None,
        },
        Step::Commit,
        Step::ExpectStatus {
            status: "queued",
            code: None,
        },
    ]);
    assert!(!agent.last().commands.is_empty());
}

/// Acceptance findings (incomplete request scope) are review notes for the
/// human boundary — they never flip check to validation_errors and never
/// block commit.
#[test]
fn acceptance_review_notes_do_not_block() {
    let fixture = &*FIXTURE;
    let store = FlowIrDraftStore::new();
    let mut agent = ScriptedAgent::new(
        &store,
        empty_board("sim-review-notes"),
        &fixture.metadata,
        "review-notes-draft",
    );
    agent.bind("Format the customer message, then send a Slack notification.");
    agent.run(vec![
        Step::Write {
            source: "eventsSimple() {\n    const formatted = stringFormat({ formatString: \"customer message\" })\n    logInfo({ message: formatted.formattedString })\n}\n"
                .to_string(),
        },
        Step::ExpectStatus {
            status: "draft_started",
            code: None,
        },
        Step::Check,
        Step::ExpectStatus {
            status: "valid",
            code: None,
        },
    ]);
    let checked = agent.last();
    assert!(
        checked.diagnostics.is_empty(),
        "review notes must not appear as blocking diagnostics: {checked:#?}"
    );
    assert!(
        checked
            .review_notes
            .iter()
            .any(|note| { note.code == FlowScriptDiagnosticCode::FsRequestAcceptanceIncomplete }),
        "incomplete request scope must surface as a review note: {checked:#?}"
    );
    assert!(
        checked.message.contains("Commit may proceed"),
        "check must tell the agent commit is not blocked: {checked:#?}"
    );

    agent.run(vec![
        Step::Commit,
        Step::ExpectStatus {
            status: "queued",
            code: None,
        },
    ]);
    let queued = agent.last();
    assert!(!queued.commands.is_empty());
    assert!(
        queued
            .review_notes
            .iter()
            .any(|note| { note.code == FlowScriptDiagnosticCode::FsRequestAcceptanceIncomplete }),
        "the queued batch must carry its review notes to the human review: {queued:#?}"
    );
}

/// Two threads race identity patches and checks against one draft. Every
/// response must be a well-formed success or revision conflict — no panics,
/// no poisoned store — and the final state must be consistent: the source is
/// unchanged and the head revision equals the number of successful patches.
#[test]
fn concurrent_same_draft() {
    let fixture = &*FIXTURE;
    let store = FlowIrDraftStore::new();
    let board = empty_board("sim-concurrent");
    let source = simple_log_script("first support step", "second support step");
    let written = store.write_flowscript(
        &board,
        &fixture.metadata,
        WriteFlowScriptArgs {
            draft_id: "concurrent-draft".to_string(),
            replace_existing: false,
            mode: FlowIrDraftMode::Additive,
            source: source.clone(),
            allow_scope_reduction: false,
        },
    );
    assert_eq!(written.status, "draft_started", "{written:#?}");

    const ITERATIONS: usize = 8;
    let successful_patches = thread::scope(|scope| {
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let store = &store;
                let board = board.clone();
                let catalog = &fixture.metadata;
                scope.spawn(move || {
                    let mut revision = 0u64;
                    let mut successes = 0usize;
                    for _ in 0..ITERATIONS {
                        let patched = store.patch_flowscript(
                            &board,
                            catalog,
                            PatchFlowScriptArgs {
                                draft_id: "concurrent-draft".to_string(),
                                expected_revision: revision,
                                // Unique in the source and replaced by itself:
                                // the revision advances, the text does not.
                                old_text: "first support step".to_string(),
                                new_text: "first support step".to_string(),
                                allow_scope_reduction: false,
                            },
                        );
                        match (patched.status.as_str(), patched.code.as_deref()) {
                            ("draft_updated", None) => successes += 1,
                            ("error", Some("FLOWSCRIPT_REVISION_CONFLICT")) => {}
                            other => panic!("unexpected patch outcome {other:?}: {patched:#?}"),
                        }
                        if let Some(current) = patched.revision {
                            revision = current;
                        }
                        let checked = store.check_flowscript(
                            &board,
                            catalog,
                            CheckFlowScriptArgs {
                                draft_id: "concurrent-draft".to_string(),
                                expected_revision: revision,
                            },
                        );
                        match (checked.status.as_str(), checked.code.as_deref()) {
                            ("valid", None) => {}
                            ("error", Some("FLOWSCRIPT_REVISION_CONFLICT")) => {}
                            other => panic!("unexpected check outcome {other:?}: {checked:#?}"),
                        }
                        if let Some(current) = checked.revision {
                            revision = current;
                        }
                    }
                    successes
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().expect("racing worker must not panic"))
            .sum::<usize>()
    });

    assert!(successful_patches >= 1, "at least one patch must land");
    // Consistent final state: head == number of successful patches, source
    // unchanged, and the draft still checks valid at that exact revision.
    let final_check = store.check_flowscript(
        &board,
        &fixture.metadata,
        CheckFlowScriptArgs {
            draft_id: "concurrent-draft".to_string(),
            expected_revision: successful_patches as u64,
        },
    );
    assert_eq!(final_check.status, "valid", "{final_check:#?}");
    assert_eq!(final_check.revision, Some(successful_patches as u64));
    assert_eq!(final_check.source.as_deref(), Some(source.as_str()));
}

/// A whole-document rewrite that renames an event and drops its anchor must claim the live entry
/// node (which already drives the body) instead of minting a duplicate. The duplicate stranded
/// the old entry with its data edges (e.g. `history` feeding body nodes), which re-reconciled to
/// a spurious ConnectPins on every readback — the exact non-idempotent board the simple-agent
/// e2e produced.
#[tokio::test]
async fn unanchored_event_rewrite_claims_live_entry() {
    let fixture = &*FIXTURE;
    let state = catalog_state().await;
    let mut board = empty_board("sim-event-rewrite");

    let authored = "eventsChat chatEvent(history: History, localSession: Struct, globalSession: Struct, tools: string[], actions: Struct[], attachments: Struct[], user: User) {\n    logInfo({ message: valToString({ value: history, pretty: true }).string })\n}\n";
    let applied = apply_flowscript_to_board(
        &mut board,
        authored,
        &fixture.nodes,
        state.clone(),
        None,
        false,
    )
    .await
    .expect("authored chat event applies");
    assert!(
        applied.diagnostics.is_empty(),
        "apply diagnostics: {:#?}",
        applied.diagnostics
    );
    let chat_entries = board
        .nodes
        .values()
        .filter(|node| node.name == "events_chat")
        .count();
    assert_eq!(chat_entries, 1, "one chat entry after the first apply");

    // The rewrite: same body anchors, renamed event, anchor dropped.
    let anchored = board_to_flowscript(
        &board,
        &RenderOptions {
            anchors: true,
            ..RenderOptions::default()
        },
    );
    let rewritten = anchored
        .lines()
        .map(|line| {
            if line.contains("eventsChat") && line.contains("//@n:") {
                let without_anchor = line.split("//@n:").next().unwrap_or(line).trim_end();
                without_anchor.replace("chatEvent", "researchAgent")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert_ne!(
        rewritten, anchored,
        "the rewrite must actually change the event line"
    );

    let result = reconcile_text_with_catalog(&board, &rewritten, &FIXTURE.metadata);
    assert!(
        result.diagnostics.is_empty(),
        "rewrite reconcile diagnostics: {:#?}\nsource:\n{rewritten}",
        result.diagnostics
    );
    let added_entries: Vec<_> = result
        .commands
        .iter()
        .filter(|command| {
            matches!(command, BoardCommand::AddNode { node_type, .. } if node_type == "events_chat")
        })
        .collect();
    assert!(
        added_entries.is_empty(),
        "the rewrite must claim the live entry, not add a duplicate: {added_entries:#?}"
    );

    let applied_rewrite =
        apply_board_commands_to_board(&mut board, result.commands, &fixture.nodes, state, None)
            .await
            .expect("rewrite commands apply");
    assert!(
        applied_rewrite.diagnostics.is_empty(),
        "rewrite apply diagnostics: {:#?}",
        applied_rewrite.diagnostics
    );
    let chat_entries_after = board
        .nodes
        .values()
        .filter(|node| node.name == "events_chat")
        .count();
    assert_eq!(
        chat_entries_after, 1,
        "still exactly one chat entry after the rewrite"
    );
    assert!(
        board
            .nodes
            .values()
            .any(|node| node.name == "events_chat" && node.friendly_name == "researchAgent"),
        "the claimed entry carries the authored name"
    );

    assert_noop_roundtrip(&board, "unanchored_event_rewrite_claims_live_entry");
}

/// Applying the golden program, lowering the applied board, and re-applying
/// the lowered text must derive zero new commands — the idempotency contract
/// that keeps FlowPilot from looping on its own successful output.
#[tokio::test]
async fn idempotent_reapply() {
    let fixture = &*FIXTURE;
    let state = catalog_state().await;
    let mut board = empty_board("sim-idempotent");

    let applied = apply_flowscript_to_board(
        &mut board,
        GOLDEN_SCRIPT,
        &fixture.nodes,
        state.clone(),
        None,
        false,
    )
    .await
    .expect("golden script applies");
    assert!(
        applied.diagnostics.is_empty(),
        "apply diagnostics: {:#?}",
        applied.diagnostics
    );
    assert!(!applied.commands.is_empty(), "first apply does real work");
    assert!(!board.nodes.is_empty());

    let anchored = board_to_flowscript(
        &board,
        &RenderOptions {
            anchors: true,
            ..RenderOptions::default()
        },
    );
    let reapplied =
        apply_flowscript_to_board(&mut board, &anchored, &fixture.nodes, state, None, false)
            .await
            .expect("lowered text re-applies");
    assert!(
        reapplied.diagnostics.is_empty(),
        "re-apply diagnostics: {:#?}\nsource:\n{anchored}",
        reapplied.diagnostics
    );
    assert!(
        reapplied.board_commands.is_empty(),
        "re-applying the board's own lowered text must derive zero commands: {:#?}\nsource:\n{anchored}",
        reapplied.board_commands
    );
    assert!(reapplied.commands.is_empty());
}

// ── Real-agent skeleton (opt-in only, never wired up implicitly) ───────

/// Placeholder for a REAL agent-driven end-to-end run. Deliberately not
/// implemented: wiring it up would require
///   1. spawning an external agent CLI (Claude Code / Codex) with the
///      FlowPilot tool bridge exposed over its tool protocol,
///   2. a funded API key plus explicit human approval per run (real runs are
///      expensive and non-deterministic),
///   3. a transcript recorder so failures can be replayed through
///      [`ScriptedAgent`] as a deterministic regression scenario.
/// Until then this skeleton only documents the contract and refuses to run
/// without the explicit `FLOWPILOT_E2E_REAL_AGENT` opt-in.
#[test]
#[ignore = "expensive: drives a real external agent; ask before running"]
fn real_agent_loop() {
    let opted_in = std::env::var("FLOWPILOT_E2E_REAL_AGENT").is_ok_and(|value| !value.is_empty());
    assert!(
        opted_in,
        "refusing to run: set FLOWPILOT_E2E_REAL_AGENT=1 to opt in to a real-agent run"
    );
    panic!(
        "not wired to a real agent yet: this skeleton needs an external agent CLI harness, \
         a funded API key, and per-run human approval before it can drive the loop for real"
    );
}
