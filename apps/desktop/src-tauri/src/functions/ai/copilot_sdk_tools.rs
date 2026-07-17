//! Copilot SDK Tool Adapters
//!
//! This module provides adapters that bridge the existing rig-based tools
//! to the Copilot SDK's tool system. The core logic is reused from
//! `flow_like::flow::copilot::tools`.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};

use super::frontend_tool_bridge::{FrontendToolApproval, FrontendToolBridge};
use super::internet_search::run_internet_search;
pub use copilot_sdk::ToolHandler;
use copilot_sdk::{Tool, ToolResultObject};
use flow_like::copilot::FlowIrCommitToken;
use flow_like::flow::ast::{
    RenderOptions, blocked_destructive_flowscript_message, board_to_flowscript,
    destructive_flowscript_command_summaries, reconcile_text_with_catalog,
};
use flow_like::flow::board::Board;
use flow_like::flow::copilot::memory::AssistantMemory;
use flow_like::flow::copilot::platform::run_memory_tool;
use flow_like::flow::copilot::tool_spec::{
    INTERNET_SEARCH_TOOL, MEMORY_SEARCH_TOOL, MEMORY_STORE_TOOL, PlatformToolSpec,
    find_global_tool_spec, global_assistant_tool_specs, missing_required_args,
    resolve_tool_approval, runtime_execution_tool_specs,
};
#[cfg(test)]
use flow_like::flow::copilot::typed_ir_schema_hint;
use flow_like::flow::copilot::{
    BeginFlowIrDraftArgs, BeginFlowIrDraftTool, BoardCommand, BoundBeginFlowIrDraftTool,
    CatalogProvider, CheckFlowScriptArgs, CheckFlowScriptTool, CommitFlowIrDraftArgs,
    CommitFlowIrDraftTool, CommitFlowScriptArgs, CommitFlowScriptTool, EmitCommandsArgs,
    FlowCapabilityPlanRequest, FlowIrAcceptanceBinding, FlowIrDraftStore,
    FlowIrRetainedDraftSnapshot, GetCurrentFlowScriptTool, GetDeclarationsArgs,
    GetDeclarationsTool, GetNodeDetailsTool, GetUnconfiguredNodesTool, GraphContext,
    ListBoardNodesTool, ModelFacingEmitCommandsTool, NodeMetadata, PatchFlowScriptArgs,
    PatchFlowScriptTool, PlanFlowIrTool, UpdateFlowIrDraftArgs, UpdateFlowIrDraftTool,
    UpsertFlowIrModuleArgs, UpsertFlowIrModuleTool, ValidateFlowIrDraftArgs,
    ValidateFlowIrDraftTool, ValidationIssue, WriteFlowScriptArgs, WriteFlowScriptTool,
    board_has_no_nodes, build_list_board_nodes_output, build_node_details_output,
    build_unconfigured_nodes_output, emit_validation_requires_flowscript,
    flowscript_has_executable_node_call, flowscript_missing_function_helpers,
    is_blocking_flowscript_diagnostic, parse_typed_ir_arguments, plan_flow_capabilities,
    render_catalog_search_results, run_declaration_queries, tool_definition_parts,
    validate_model_facing_emit_commands, validate_model_facing_emit_commands_scope,
};
use flow_like_types::sync::Mutex as AsyncMutex;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

const FLOW_IR_DRAFT_STORE_TTL: Duration = Duration::from_secs(45 * 60);
const FLOW_IR_PENDING_REVIEW_TTL: Duration = Duration::from_secs(2 * 60 * 60);
const MAX_PERSISTED_FLOW_IR_DRAFT_STORES: usize = 64;

struct CachedFlowIrDraftStore {
    store: Arc<FlowIrDraftStore>,
    last_accessed: Instant,
    /// Absolute lease start for an unresolved native review. Unlike `last_accessed`, this is not
    /// refreshed by preflight or other token traffic, so a response lost after native finalization
    /// cannot pin one of the bounded board-store slots forever.
    pending_since: Option<Instant>,
}

/// A commit claim stays attached to its queued command batch until the desktop host actually
/// drains that batch into a response/event. Dropping an undrained queue (cancellation, host error,
/// or surface teardown) reopens only the exact claimed revision so it can be retried safely.
struct PendingRetainedCommitClaim {
    store: Arc<FlowIrDraftStore>,
    token: FlowIrCommitToken,
    queued_flowscript: Option<Arc<Mutex<Option<String>>>>,
    flowscript: Option<String>,
    expected_command_count: usize,
    acknowledged: bool,
}

impl PendingRetainedCommitClaim {
    fn new(
        store: Arc<FlowIrDraftStore>,
        token: FlowIrCommitToken,
        queued_flowscript: Option<Arc<Mutex<Option<String>>>>,
        flowscript: Option<String>,
        expected_command_count: usize,
    ) -> Self {
        Self {
            store,
            token,
            queued_flowscript,
            flowscript,
            expected_command_count,
            acknowledged: false,
        }
    }

    fn acknowledge(mut self) -> FlowIrCommitToken {
        self.acknowledged = true;
        self.token.clone()
    }
}

fn release_retained_commit_claim(store: &FlowIrDraftStore, token: &FlowIrCommitToken) -> bool {
    store.release_commit_if_matches(
        &token.draft_id,
        token.revision,
        &token.base_fingerprint,
        &token.claim_id,
    )
}

impl Drop for PendingRetainedCommitClaim {
    fn drop(&mut self) {
        if !self.acknowledged {
            release_retained_commit_claim(&self.store, &self.token);
            if let (Some(workspace), Some(flowscript)) =
                (&self.queued_flowscript, self.flowscript.as_deref())
                && let Ok(mut queued) = workspace.lock()
                && queued.as_deref() == Some(flowscript)
            {
                *queued = None;
            }
        }
    }
}

/// Commands produced by host-local tools, including the retained-draft claim that authorizes an
/// exact checked batch. Plain direct-command and legacy FlowScript tools leave `commit_claim`
/// empty.
#[derive(Default)]
pub(super) struct SideEffectCommandQueue {
    commands: Vec<BoardCommand>,
    commit_claim: Option<PendingRetainedCommitClaim>,
}

impl SideEffectCommandQueue {
    fn extend(&mut self, commands: impl IntoIterator<Item = BoardCommand>) -> bool {
        if self.commit_claim.is_some() {
            return false;
        }
        self.commands.extend(commands);
        true
    }

    fn extend_retained_commit(
        &mut self,
        commands: impl IntoIterator<Item = BoardCommand>,
        store: Arc<FlowIrDraftStore>,
        token: FlowIrCommitToken,
        queued_flowscript: Option<Arc<Mutex<Option<String>>>>,
        flowscript: Option<String>,
    ) -> bool {
        let commands = commands.into_iter().collect::<Vec<_>>();
        let claim = PendingRetainedCommitClaim::new(
            store,
            token,
            queued_flowscript,
            flowscript,
            commands.len(),
        );
        if self.commit_claim.is_some() || !self.commands.is_empty() {
            // A response has room for one exact review token and its exact retained batch. Never
            // attach that token to unrelated direct commands already waiting in the queue.
            // Dropping the new claim reopens its revision without disturbing the earlier batch.
            drop(claim);
            return false;
        }
        self.commands.extend(commands);
        self.commit_claim = Some(claim);
        true
    }

    /// Direct/legacy commands may be streamed before the response is finalized. A retained commit
    /// is indivisible: while its claim exists, keep the full command batch in this queue so no
    /// caller can expose commands before it can atomically take their exact review token.
    pub(super) fn drain_streamable(&mut self) -> Vec<BoardCommand> {
        if self.commit_claim.is_some() {
            return Vec::new();
        }
        self.commands.drain(..).collect()
    }

    /// Atomically transfer the final command tail and its exact Apply/Dismiss token under one host
    /// lock. A claimed batch whose command count changed is malformed and fails closed by
    /// reopening the retained revision instead of returning a token for a different batch.
    pub(super) fn take_delivery(&mut self) -> (Vec<BoardCommand>, Option<FlowIrCommitToken>) {
        if self.commit_claim.as_ref().is_some_and(|claim| {
            self.commands.is_empty() || self.commands.len() != claim.expected_command_count
        }) {
            self.abandon();
            return (Vec::new(), None);
        }
        let commands = self.commands.drain(..).collect();
        let token = self.commit_claim.take().map(|claim| claim.acknowledge());
        (commands, token)
    }

    /// Fail closed after a poisoned queue: discard commands and let pending claim drops reopen the
    /// retained revisions instead of reporting an idempotent success for a batch nobody received.
    pub(super) fn abandon(&mut self) {
        self.commands.clear();
        self.commit_claim = None;
    }
}

/// Retained workflow drafts need to survive provider continuations and a fresh SDK/MCP surface for
/// the same board. Keep them board-scoped, time-bounded, and capped so reopening tools does not
/// silently discard repairable work or turn the process cache into an unbounded session store.
static FLOW_IR_DRAFT_STORES: LazyLock<Mutex<HashMap<String, CachedFlowIrDraftStore>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, Copy)]
enum FlowIrDraftStoreAccessError {
    BoardIdRequired,
    Capacity,
    EpochChanged,
    Unavailable,
}

impl FlowIrDraftStoreAccessError {
    fn tool_result(self) -> ToolResultObject {
        let (code, message, retryable, next_action) = match self {
            Self::BoardIdRequired => (
                "IR_DRAFT_BOARD_ID_REQUIRED",
                "Retained FlowScript drafts require a persistent board id so Apply/Dismiss can resolve their exact commit token.",
                false,
                "use_a_persisted_board",
            ),
            Self::Capacity => (
                "IR_DRAFT_STORE_CAPACITY",
                "Retained draft capacity is occupied by unresolved board reviews. Apply or dismiss an existing FlowPilot review before starting one for another board.",
                true,
                "resolve_pending_review",
            ),
            Self::EpochChanged => (
                "FLOWSCRIPT_DRAFT_STORE_EPOCH_CHANGED",
                "This tool surface's immutable request binding belongs to an older board-draft store epoch. Do not retry this tool call in the current phase; restart the FlowPilot phase so the host can bind the same request to the active store.",
                false,
                "restart_agent_phase",
            ),
            Self::Unavailable => (
                "IR_DRAFT_STORE_UNAVAILABLE",
                "The board-scoped retained draft cache is unavailable. No detached draft store was created; retry after the host recovers.",
                true,
                "retry_after_host_recovery",
            ),
        };
        ToolResultObject::text(
            json!({
                "status": "error",
                "code": code,
                "retryable": retryable,
                "next_action": next_action,
                "message": message,
            })
            .to_string(),
        )
    }
}

fn flow_ir_draft_store_for_board(
    board: &Board,
    live_board: Option<&Arc<AsyncMutex<Board>>>,
) -> Result<Arc<FlowIrDraftStore>, FlowIrDraftStoreAccessError> {
    with_current_board(board, live_board, |current| {
        persisted_flow_ir_draft_store(&board.id, current)
    })
}

/// Acquire the same retained board-scoped store for the built-in Bits/core path. External SDK
/// tools and the core rig loop must share this cache because the native atomic Apply endpoint
/// resolves review tokens here after the originating chat request has returned.
pub(super) fn retained_flow_ir_draft_store_for_board(
    board: &Board,
) -> Result<Arc<FlowIrDraftStore>, String> {
    persisted_flow_ir_draft_store(&board.id, board).map_err(|error| match error {
        FlowIrDraftStoreAccessError::BoardIdRequired => {
            "Retained FlowScript drafts require a persistent board id so Apply/Dismiss can resolve their exact commit token.".to_string()
        }
        FlowIrDraftStoreAccessError::Capacity => {
            "Retained draft capacity is occupied by unresolved board reviews. Apply or dismiss an existing FlowPilot review before starting one for another board.".to_string()
        }
        FlowIrDraftStoreAccessError::EpochChanged => {
            "The board-scoped retained draft store changed while constructing this tool surface; restart the FlowPilot phase.".to_string()
        }
        FlowIrDraftStoreAccessError::Unavailable => {
            "The board-scoped retained draft cache is unavailable; no detached draft store was created.".to_string()
        }
    })
}

fn persisted_flow_ir_draft_store(
    board_key: &str,
    observed_board: &Board,
) -> Result<Arc<FlowIrDraftStore>, FlowIrDraftStoreAccessError> {
    touch_persisted_flow_ir_draft_store(
        board_key,
        Arc::new(FlowIrDraftStore::new()),
        Some(observed_board),
    )
}

/// Refresh a board-scoped draft-store lease at retained-tool execution time, not merely when the
/// tool surface is constructed. If a newer surface already installed a store, all callers converge
/// on that store rather than reviving a detached one.
fn touch_persisted_flow_ir_draft_store(
    board_key: &str,
    fallback: Arc<FlowIrDraftStore>,
    observed_board: Option<&Board>,
) -> Result<Arc<FlowIrDraftStore>, FlowIrDraftStoreAccessError> {
    if board_key.trim().is_empty() {
        return Err(FlowIrDraftStoreAccessError::BoardIdRequired);
    }
    let now = Instant::now();
    let mut stores = FLOW_IR_DRAFT_STORES
        .lock()
        .map_err(|_| FlowIrDraftStoreAccessError::Unavailable)?;
    // Observation is diagnostic only. Pending review tokens are released exclusively by explicit
    // Apply/Dismiss disposition so unrelated board edits cannot silently acknowledge a batch.
    if let (Some(board), Some(cached)) = (observed_board, stores.get(board_key)) {
        cached.store.observe_board(board);
    }
    prune_expired_flow_ir_draft_stores(&mut stores, now);
    if let Some(cached) = stores.get_mut(board_key) {
        cached.last_accessed = now;
        return Ok(cached.store.clone());
    }
    // Never split a board's store while it carries an unresolved review. Reclaim an idle lease;
    // when all slots are pending, fail closed rather than creating an untracked ephemeral store.
    if !reclaim_flow_ir_draft_store_slot(&mut stores) {
        return Err(FlowIrDraftStoreAccessError::Capacity);
    }
    let store = fallback;
    if let Some(board) = observed_board {
        hydrate_flow_ir_draft_store_from_disk(board_key, board, &store);
    }
    stores.insert(
        board_key.to_string(),
        CachedFlowIrDraftStore {
            store: store.clone(),
            last_accessed: now,
            pending_since: None,
        },
    );
    Ok(store)
}

fn touch_bound_flowscript_draft_store(
    board_key: &str,
    bound_store: Arc<FlowIrDraftStore>,
) -> Result<Arc<FlowIrDraftStore>, FlowIrDraftStoreAccessError> {
    let active = touch_persisted_flow_ir_draft_store(board_key, bound_store.clone(), None)?;
    if Arc::ptr_eq(&active, &bound_store) {
        Ok(active)
    } else {
        Err(FlowIrDraftStoreAccessError::EpochChanged)
    }
}

fn reclaim_flow_ir_draft_store_slot(stores: &mut HashMap<String, CachedFlowIrDraftStore>) -> bool {
    prune_expired_flow_ir_draft_stores(stores, Instant::now());
    if stores.len() < MAX_PERSISTED_FLOW_IR_DRAFT_STORES {
        return true;
    }
    if let Some(oldest_key) = stores
        .iter()
        .filter(|(_, cached)| !cached.store.has_pending_commit())
        .min_by_key(|(_, cached)| cached.last_accessed)
        .map(|(key, _)| key.clone())
    {
        stores.remove(&oldest_key);
    }
    stores.len() < MAX_PERSISTED_FLOW_IR_DRAFT_STORES
}

fn prune_expired_flow_ir_draft_stores(
    stores: &mut HashMap<String, CachedFlowIrDraftStore>,
    now: Instant,
) {
    stores.retain(|_, cached| {
        if cached.store.has_pending_commit() {
            // `last_accessed` is refreshed immediately before every retained tool call, including
            // the commit that creates the pending claim. Use that timestamp when first observing
            // the claim so its absolute lease starts no later than commit time.
            let pending_since = *cached.pending_since.get_or_insert(cached.last_accessed);
            now.saturating_duration_since(pending_since) <= FLOW_IR_PENDING_REVIEW_TTL
        } else {
            cached.pending_since = None;
            now.saturating_duration_since(cached.last_accessed) <= FLOW_IR_DRAFT_STORE_TTL
        }
    });
}

/// Resolve an existing board-scoped store for Apply/Dismiss lifecycle commands without ever
/// creating a replacement. Pending stores survive the normal idle TTL but expire at the absolute
/// review lease, so a missing entry means the token cannot be proven against this desktop process.
pub(super) fn retained_flow_ir_draft_store(board_key: &str) -> Option<Arc<FlowIrDraftStore>> {
    let mut stores = FLOW_IR_DRAFT_STORES.lock().ok()?;
    let now = Instant::now();
    prune_expired_flow_ir_draft_stores(&mut stores, now);
    let cached = stores.get_mut(board_key.trim())?;
    cached.last_accessed = now;
    let store = cached.store.clone();
    drop(stores);
    // Apply/Dismiss mutate this store right after resolving it; the debounced snapshot below runs
    // after those dispositions and therefore captures the post-disposition draft state.
    schedule_flow_ir_draft_snapshot(board_key, &store);
    Some(store)
}

const FLOW_IR_DRAFT_SNAPSHOT_DEBOUNCE: Duration = Duration::from_millis(500);

static FLOW_IR_DRAFT_SNAPSHOT_GENERATIONS: LazyLock<Mutex<HashMap<String, u64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Root for on-disk retained-draft snapshots.
///
/// Secret hygiene: draft FlowScript sources can carry `@secret` consts and other request-derived
/// values, so snapshots require exactly the protection level of the boards themselves. Boards
/// already persist locally — variables included — under the settings project root, which defaults
/// to `{data_dir}/flow-like/projects`; snapshots mirror that root instead of a shared temp or
/// cache directory and never a location that syncs off-device. `FLOW_LIKE_FLOWPILOT_DRAFT_DIR`
/// overrides the root for tests.
fn flow_ir_draft_snapshot_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("FLOW_LIKE_FLOWPILOT_DRAFT_DIR") {
        let dir = PathBuf::from(dir);
        return (!dir.as_os_str().is_empty()).then_some(dir);
    }
    #[cfg(test)]
    {
        // Unit tests exercise the retained tool handlers directly; keep their snapshots out of
        // the user's real data dir and isolated per test process.
        static TEST_SNAPSHOT_DIR: LazyLock<PathBuf> = LazyLock::new(|| {
            std::env::temp_dir().join(format!(
                "flow-like-test-flowpilot-drafts-{}",
                std::process::id()
            ))
        });
        Some(TEST_SNAPSHOT_DIR.clone())
    }
    #[cfg(not(test))]
    Some(
        dirs_next::data_dir()?
            .join("flow-like")
            .join("projects")
            .join(".flowpilot-drafts"),
    )
}

fn flow_ir_draft_snapshot_path(board_key: &str) -> Option<PathBuf> {
    let board_key = board_key.trim();
    if board_key.is_empty() {
        return None;
    }
    let sanitized = board_key
        .chars()
        .take(64)
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    // FNV-1a over the untruncated key keeps sanitized/truncated board keys collision-free.
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in board_key.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    Some(flow_ir_draft_snapshot_dir()?.join(format!("{sanitized}-{hash:016x}.drafts.json")))
}

/// Debounced crash-durability write for a board's retained drafts. Every draft-mutating tool call
/// schedules one; only the newest generation writes, so a burst of tool calls produces one file.
fn schedule_flow_ir_draft_snapshot(board_key: &str, store: &Arc<FlowIrDraftStore>) {
    let board_key = board_key.trim().to_string();
    let Some(path) = flow_ir_draft_snapshot_path(&board_key) else {
        return;
    };
    let Ok(mut generations) = FLOW_IR_DRAFT_SNAPSHOT_GENERATIONS.lock() else {
        return;
    };
    let generation = generations
        .entry(board_key.clone())
        .and_modify(|generation| *generation = generation.wrapping_add(1))
        .or_insert(1);
    let generation = *generation;
    drop(generations);
    let store = store.clone();
    std::thread::spawn(move || {
        std::thread::sleep(FLOW_IR_DRAFT_SNAPSHOT_DEBOUNCE);
        let is_current = FLOW_IR_DRAFT_SNAPSHOT_GENERATIONS
            .lock()
            .is_ok_and(|generations| generations.get(&board_key) == Some(&generation));
        if is_current {
            persist_flow_ir_draft_snapshot(&path, &store);
        }
    });
}

/// Atomic-rename snapshot write. An empty snapshot removes the file so applied or dismissed
/// drafts do not linger on disk after their session resolves.
fn persist_flow_ir_draft_snapshot(path: &Path, store: &FlowIrDraftStore) {
    let snapshot = store.export_retained_snapshot();
    if snapshot.is_empty() {
        let _ = std::fs::remove_file(path);
        return;
    }
    let Ok(encoded) = serde_json::to_vec(&snapshot) else {
        return;
    };
    let Some(parent) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(parent).is_err() {
        return;
    }
    let temp = path.with_extension("json.tmp");
    if std::fs::write(&temp, encoded).is_ok() {
        let _ = std::fs::rename(&temp, path);
    }
}

/// Restore crash-durable drafts into a freshly created board store. The core import is fail-safe:
/// entries whose board fingerprint no longer matches the live board are skipped, never revived.
fn hydrate_flow_ir_draft_store_from_disk(board_key: &str, board: &Board, store: &FlowIrDraftStore) {
    let Some(path) = flow_ir_draft_snapshot_path(board_key) else {
        return;
    };
    hydrate_flow_ir_draft_store_from_path(&path, board, store);
}

fn hydrate_flow_ir_draft_store_from_path(path: &Path, board: &Board, store: &FlowIrDraftStore) {
    let Ok(encoded) = std::fs::read(path) else {
        return;
    };
    let Ok(snapshot) = serde_json::from_slice::<FlowIrRetainedDraftSnapshot>(&encoded) else {
        return;
    };
    store.import_retained_snapshot(board, snapshot);
}

/// Hold the registry-backed board lock for the entire operation so fingerprint validation and
/// command queueing observe one host state. Detached/anonymous boards retain the captured fallback.
fn with_current_board<T>(
    captured: &Board,
    live: Option<&Arc<AsyncMutex<Board>>>,
    operation: impl FnOnce(&Board) -> T,
) -> T {
    match live {
        Some(live) => {
            let board = block_on_tool(live.lock());
            operation(&board)
        }
        None => operation(captured),
    }
}

/// Create all Copilot SDK tools for board context.
///
/// When a live `board` and immutable edit request are supplied, the code-first FlowScript surface
/// is enabled: `write_flowscript` retains the complete source, `patch_flowscript` repairs an exact
/// revision, `check_flowscript` retains the compiler-derived command batch, and
/// `commit_flowscript` transfers only that exact batch into the normal Apply/Dismiss boundary.
/// Legacy typed-JSON and one-shot `edit_flowscript` adapters remain implemented below for old
/// callers, but are intentionally not advertised to model-facing SDK/MCP surfaces.
pub(super) fn create_board_tools(
    graph_context: Option<Arc<GraphContext>>,
    board: Option<Arc<Board>>,
    live_board: Option<Arc<AsyncMutex<Board>>>,
    request_acceptance_prompt: Option<&str>,
    catalog_provider: Option<Arc<dyn CatalogProvider>>,
    side_effect_commands: Option<Arc<Mutex<SideEffectCommandQueue>>>,
    queued_flowscript: Option<Arc<Mutex<Option<String>>>>,
) -> Vec<(Tool, ToolHandler)> {
    let mut tools = vec![
        create_catalog_search_tool(catalog_provider.clone()),
        create_emit_commands_tool(
            graph_context.clone(),
            catalog_provider.clone(),
            side_effect_commands.clone(),
        ),
    ];

    if let Some(provider) = catalog_provider.clone() {
        tools.push(create_get_declarations_tool(provider.clone()));
    }

    if let Some(board) = board {
        let flow_ir_drafts = flow_ir_draft_store_for_board(&board, live_board.as_ref())
            // This value only supplies tool schemas if capacity is currently exhausted. Every
            // invocation reacquires the board lease and returns the explicit host error first.
            .unwrap_or_else(|_| Arc::new(FlowIrDraftStore::new()));
        let acceptance_binding = request_acceptance_prompt
            .map(|prompt| flow_ir_drafts.bind_request_acceptance_contract(&board.id, prompt));
        tools.push(create_get_current_flowscript_tool(
            board.clone(),
            live_board.clone(),
        ));
        if let (Some(provider), Some(acceptance_binding)) =
            (catalog_provider.clone(), acceptance_binding)
        {
            tools.push(create_write_flowscript_tool(
                board.clone(),
                live_board.clone(),
                provider.clone(),
                flow_ir_drafts.clone(),
                acceptance_binding.clone(),
            ));
            tools.push(create_patch_flowscript_tool(
                board.clone(),
                live_board.clone(),
                provider.clone(),
                flow_ir_drafts.clone(),
                acceptance_binding.clone(),
            ));
            tools.push(create_check_flowscript_tool(
                board.clone(),
                live_board.clone(),
                provider.clone(),
                flow_ir_drafts.clone(),
                acceptance_binding.clone(),
            ));
            tools.push(create_commit_flowscript_tool(
                board,
                live_board,
                provider,
                flow_ir_drafts,
                acceptance_binding,
                side_effect_commands.clone(),
                queued_flowscript.clone(),
            ));
        }
    }

    if let Some(ctx) = graph_context.clone() {
        tools.push(create_get_node_details_tool(ctx));
    }

    if let Some(ctx) = graph_context.clone() {
        tools.push(create_get_unconfigured_nodes_tool(ctx));
    }

    if let Some(ctx) = graph_context {
        tools.push(create_list_board_nodes_tool(ctx));
    }

    tools
}

fn typed_draft_request_access_denied(
    store: &FlowIrDraftStore,
    board_id: &str,
    draft_id: &str,
    acceptance_binding: Option<&FlowIrAcceptanceBinding>,
) -> Option<ToolResultObject> {
    store
        .authorize_draft_request(board_id, draft_id, acceptance_binding)
        .err()
        .map(|denied| {
            ToolResultObject::text(
                serde_json::to_string_pretty(&denied).unwrap_or_else(|error| {
                    json!({
                        "status": "request_identity_mismatch",
                        "code": "IR_DRAFT_REQUEST_IDENTITY_MISMATCH",
                        "retryable": false,
                        "auto_resume": false,
                        "message": format!("Failed to render request recovery metadata: {error}")
                    })
                    .to_string()
                }),
            )
        })
}

/// Create runtime tools that execute through the frontend bridge.
///
/// These tools need browser/app context such as the active backend state, storage provider,
/// approval dialogs, and execution service. The Rust SDK tool blocks until the frontend replies.
pub fn create_runtime_tools(bridge: FrontendToolBridge) -> Vec<(Tool, ToolHandler)> {
    let mut tools = vec![
        create_database_tool(bridge.clone()),
        create_storage_tool(bridge.clone()),
        create_ui_inspect_tool(bridge.clone()),
    ];
    tools.extend(
        runtime_execution_tool_specs()
            .iter()
            .map(|spec| sdk_tool_from_spec(spec, bridge.clone(), None)),
    );
    for name in [INTERNET_SEARCH_TOOL, "ask_user"] {
        if let Some(spec) = find_global_tool_spec(name) {
            tools.push(sdk_tool_from_spec(&spec, bridge.clone(), None));
        }
    }
    tools
}

/// Adapt one shared platform tool spec to the Copilot SDK tool type.
///
/// Execution funnels through the frontend bridge with the spec's approval + timeout, except the
/// host-local tools: `internet_search` runs in-process and the `_memory_*` tools run against the
/// profile's `AssistantMemory`.
pub fn sdk_tool_from_spec(
    spec: &PlatformToolSpec,
    bridge: FrontendToolBridge,
    memory: Option<Arc<AssistantMemory>>,
) -> (Tool, ToolHandler) {
    let tool = Tool::new(spec.name)
        .description(spec.description)
        .schema((spec.schema)());
    let spec = *spec;
    let handler: ToolHandler = Arc::new(move |_name, args| {
        if let Some(error) = missing_required_args(&spec, args) {
            return ToolResultObject::text(
                json!({ "status": "error", "error": error }).to_string(),
            );
        }
        match spec.name {
            INTERNET_SEARCH_TOOL => ToolResultObject::text(
                serde_json::to_string_pretty(&run_blocking_tool(|| run_internet_search(args)))
                    .unwrap_or_else(|_| "{\"status\":\"error\"}".to_string()),
            ),
            MEMORY_STORE_TOOL | MEMORY_SEARCH_TOOL => ToolResultObject::text(block_on_tool(
                run_memory_tool(spec.name, args, memory.as_deref()),
            )),
            _ => frontend_tool_result_with_timeout(
                &bridge,
                spec.name,
                args.clone(),
                approval_from_spec(&spec, args),
                Duration::from_secs(spec.timeout_secs),
            ),
        }
    });
    (tool, handler)
}

pub fn approval_from_spec(spec: &PlatformToolSpec, args: &Value) -> FrontendToolApproval {
    // Approval policy is resolved by core so the desktop (Tauri event) and the browser (SSE frame)
    // enforce exactly the same rules; here it's just wrapped in the desktop's serialize type.
    let resolved = resolve_tool_approval(spec, args);
    FrontendToolApproval {
        kind: resolved.kind,
        title: resolved.title,
        description: resolved.description,
        session_key: resolved.session_key,
    }
}

/// Copilot SDK tool handlers are synchronous and invoked on the async runtime; run the async
/// host-local tools without stalling sibling tasks.
fn block_on_tool<F: std::future::Future>(future: F) -> F::Output {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(future)),
        Err(_) => tauri::async_runtime::block_on(future),
    }
}

/// Run a blocking tool body without pinning the runtime worker it is called on.
///
/// The Copilot SDK invokes tool handlers synchronously on a runtime worker thread. A handler that
/// blocks — network I/O (`internet_search`) or waiting on the frontend bridge (`list_apps`, …) —
/// would pin that worker; with parallel tool calls that starves the runtime the frontend round-trip
/// itself needs, deadlocking the batch. `block_in_place` hands this worker's other tasks to a
/// sibling so the runtime keeps progressing while the closure blocks.
fn run_blocking_tool<T>(f: impl FnOnce() -> T) -> T {
    match tokio::runtime::Handle::try_current() {
        Ok(_) => tokio::task::block_in_place(f),
        Err(_) => f(),
    }
}

/// Tool set for the global FlowPilot assistant, generated from the shared platform tool specs so
/// every backend (Bits/rig, GitHub Copilot, Codex, Claude Code) advertises identical tools.
/// App-scoped data/storage tools and the board-scoped `execute_event` alias are excluded because
/// the global assistant is not bound to a single app. Global execution uses `call_app_event`, while
/// `execute_node` and `query_execution_logs` require explicit app/board/run ids in their schemas.
pub fn create_global_assistant_tools(
    bridge: FrontendToolBridge,
    memory: Option<Arc<AssistantMemory>>,
) -> Vec<(Tool, ToolHandler)> {
    global_assistant_tool_specs(memory.is_some())
        .iter()
        .map(|spec| sdk_tool_from_spec(spec, bridge.clone(), memory.clone()))
        .collect()
}

fn frontend_tool_result(
    bridge: &FrontendToolBridge,
    tool_name: &'static str,
    args: Value,
    approval: FrontendToolApproval,
) -> ToolResultObject {
    frontend_tool_result_with_timeout(bridge, tool_name, args, approval, Duration::from_secs(120))
}

fn frontend_tool_result_with_timeout(
    bridge: &FrontendToolBridge,
    tool_name: &'static str,
    args: Value,
    approval: FrontendToolApproval,
    timeout: Duration,
) -> ToolResultObject {
    let result = run_blocking_tool(|| bridge.call_with_timeout(tool_name, args, approval, timeout));
    ToolResultObject::text(
        serde_json::to_string_pretty(&result)
            .unwrap_or_else(|_| "{\"status\":\"error\"}".to_string()),
    )
}

fn arg_string(args: &Value, snake: &str, camel: &str) -> String {
    args.get(snake)
        .or_else(|| args.get(camel))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn database_operation_requires_approval(operation: &str) -> bool {
    matches!(
        operation,
        "create_table"
            | "insert"
            | "add_items"
            | "delete"
            | "remove_items"
            | "update"
            | "build_index"
            | "drop_index"
            | "optimize"
            | "add_column"
            | "drop_columns"
            | "alter_column"
    )
}

fn flowscript_validation_message(flowscript: &str, diagnostics: &[String]) -> String {
    let missing_function_helpers = flowscript_missing_function_helpers(flowscript, diagnostics);
    if !missing_function_helpers.is_empty() {
        return format!(
            "FlowScript validation failed: local helper declaration(s) {} are missing the required `function` keyword. Write `function helperName(...) {{ ... }}`; these are local Function layers, not catalog nodes, so another declaration search will not fix them.",
            missing_function_helpers
                .iter()
                .map(|name| format!("`{name}`"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    if diagnostics.iter().any(|diagnostic| {
        diagnostic.contains("return value")
            && diagnostic.contains("no matching function return pin")
    }) {
        return "FlowScript validation failed: a helper returns a value without declaring a matching output pin. Add a named return signature, for example `function classify(body: string): (isSupport: bool) { ...; return result.value }`.".to_string();
    }

    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.contains("nodes (max"))
    {
        return "FlowScript validation failed: a layer would exceed the 50-node cap. Nothing was queued. Split the logic into smaller `function name(...) { ... }` declarations — each function layer has its own 50-node budget — and call the helpers from the parent flow.".to_string();
    }

    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.contains("labelled branch requires a call condition"))
    {
        return "FlowScript validation failed: labelled branch syntax (`if (...) { // label ... }`) requires the condition to be a catalog/control-node call. For ordinary boolean checks, remove the trailing branch labels/comments and use plain `if (condition) { ... } else { ... }`, or use exact control-node declarations from get_declarations.".to_string();
    }

    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.contains("expected `Colon`, found `Assign`"))
    {
        return "FlowScript validation failed: object and call-argument fields use colon syntax, for example `{ host: \"imap.gmail.com\" }`, not assignment syntax like `{ host = \"imap.gmail.com\" }`.".to_string();
    }

    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.contains("`const` binding requires a call expression"))
    {
        return "FlowScript validation failed: inside a function/event block, `const name = ...` must bind a catalog/node call. Use `let` for local literal aliases or pass literals/objects directly into node calls.".to_string();
    }

    "FlowScript validation failed. Fix the listed issues and call edit_flowscript again."
        .to_string()
}

fn flowscript_summary(flowscript: &str) -> Value {
    json!({
        "lines": if flowscript.is_empty() { 0 } else { flowscript.lines().count() },
        "chars": flowscript.chars().count(),
    })
}

fn create_database_tool(bridge: FrontendToolBridge) -> (Tool, ToolHandler) {
    let tool = Tool::new("database_tool")
        .description(
            r#"Inspect or modify the app's built-in LanceDB/Open Database tables through the frontend backend state.

Use this to understand existing local/user databases before generating DataFusion, Lance, vector,
full-text, or hybrid search workflows.

Read operations do not ask for approval. Mutating operations show an approval dialog with a
"don't ask again this session" option.

Operations:
- list_tables: return project and user-scoped tables.
- create_table: create an empty table from explicit fields [{name,type,nullable?,vector_size?}].
  `if_not_exists` defaults to true; no seed row is inserted. A `partial` result with
  `explicit_schema_create_not_deployed` means the remote API is older than this client: retain the
  schema request and continue the workflow build instead of switching to a smoke test.
- describe_table: schema, indices, row count, and sample rows.
- query: SQL/filter/vector/FTS query via the existing database query API.
- insert/add_items, delete/remove_items, update.
- build_index, drop_index, optimize, add_column, drop_columns, alter_column."#,
        )
        .schema(json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": [
                        "list_tables", "create_table", "describe_table", "query",
                        "insert", "add_items", "delete", "remove_items", "update",
                        "build_index", "drop_index", "optimize",
                        "add_column", "drop_columns", "alter_column"
                    ]
                },
                "app_id": { "type": "string", "description": "App id. Optional when FlowPilot knows the current app." },
                "table_name": { "type": "string", "description": "Table name for table operations." },
                "user_scoped": { "type": "boolean", "description": "Use user-scoped storage/database tables." },
                "fields": {
                    "type": "array",
                    "description": "Explicit fields for create_table. Supported types: string, boolean, int8/int16/int32/int64, uint8/uint16/uint32/uint64, float32/float64, binary, date32, timestamp, vector. Vector fields require vector_size.",
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "name": { "type": "string" },
                            "type": { "type": "string" },
                            "nullable": { "type": "boolean", "description": "Defaults to true." },
                            "vector_size": { "type": "integer", "minimum": 1 }
                        },
                        "required": ["name", "type"]
                    }
                },
                "if_not_exists": { "type": "boolean", "description": "For create_table, succeed if the table already exists. Defaults to true." },
                "query": { "type": "object", "description": "Query payload: {sql, filter, fts_term, vector_query, rerank}." },
                "offset": { "type": "integer" },
                "limit": { "type": "integer" },
                "items": { "type": "array", "items": { "type": "object" } },
                "filter": { "type": "string", "description": "Delete/update filter expression." },
                "updates": { "type": "object" },
                "column": { "type": "string" },
                "columns": { "type": "array", "items": { "type": "string" } },
                "index_type": {
                    "type": "string",
                    "enum": ["FullText", "BTree", "Bitmap", "LabelList", "Auto", "full_text", "btree", "bitmap", "label_list", "auto"]
                },
                "index_name": { "type": "string" },
                "optimize": { "type": "boolean" },
                "keep_versions": { "type": "boolean" },
                "nullable": { "type": "boolean" },
                "column_definition": { "type": "object", "description": "For add_column: {name, sql_expression}." }
            },
            "required": ["operation"]
        }));

    let handler: ToolHandler = Arc::new(move |_name, args| {
        let operation = arg_string(args, "operation", "operation");
        let approval = if database_operation_requires_approval(&operation) {
            let table_name = arg_string(args, "table_name", "tableName");
            FrontendToolApproval::mutating(
                "Approve database change",
                format!(
                    "FlowPilot wants to run database operation '{}'{}.",
                    operation,
                    if table_name.is_empty() {
                        String::new()
                    } else {
                        format!(" on table '{table_name}'")
                    }
                ),
                format!("database:{operation}"),
            )
        } else {
            FrontendToolApproval::none()
        };
        frontend_tool_result(&bridge, "database_tool", args.clone(), approval)
    });

    (tool, handler)
}

fn create_storage_tool(bridge: FrontendToolBridge) -> (Tool, ToolHandler) {
    let tool = Tool::new("storage_tool")
        .description(
            r#"List, read, create, or delete app storage files through the frontend storage state.

Read/list operations are silent. create_file and delete_files show an approval dialog with a
"don't ask again this session" option. Use this when a workflow needs to reference existing files
or create a small helper/config artifact in app/user storage."#,
        )
        .schema(json!({
            "type": "object",
            "properties": {
                "operation": { "type": "string", "enum": ["list_files", "read_file", "create_file", "delete_files"] },
                "app_id": { "type": "string", "description": "App id. Optional when FlowPilot knows the current app." },
                "prefix": { "type": "string", "description": "Folder/prefix to list." },
                "path": { "type": "string", "description": "File path for read/create." },
                "paths": { "type": "array", "items": { "type": "string" }, "description": "File paths/prefixes for deletion." },
                "content": { "type": "string", "description": "Text content for create_file." },
                "mime_type": { "type": "string", "description": "Content type for create_file, default text/plain." },
                "user_scoped": { "type": "boolean", "description": "Use user storage instead of app storage." },
                "max_chars": { "type": "integer", "description": "Maximum characters to return for read_file." }
            },
            "required": ["operation"]
        }));

    let handler: ToolHandler = Arc::new(move |_name, args| {
        let operation = arg_string(args, "operation", "operation");
        let approval = if matches!(operation.as_str(), "create_file" | "delete_files") {
            FrontendToolApproval::mutating(
                "Approve storage change",
                format!("FlowPilot wants to run storage operation '{operation}'."),
                format!("storage:{operation}"),
            )
        } else {
            FrontendToolApproval::none()
        };
        frontend_tool_result(&bridge, "storage_tool", args.clone(), approval)
    });

    (tool, handler)
}

fn create_ui_inspect_tool(bridge: FrontendToolBridge) -> (Tool, ToolHandler) {
    let tool = Tool::new("ui_inspect")
        .description(
            r#"Inspect the app's A2UI pages and widgets so `a2ui*` workflow calls target real elements.

This is a READ-ONLY tool and never asks for approval. Call it BEFORE writing or editing any
`a2ui*` call (set/get element, instantiate widget, push/clear container, navigate) so element
references and widget selectors are never guessed.

Operations:
- list (default): every page (id, name, route, onLoad event) and every widget (selector, description).
- page: full element reference list for one page. An `elementRef` used by `a2uiSetElementText`,
  `a2uiGetElement`, `a2uiGetElementValue`, `a2uiPushToContainer`, etc. is `"<page_id>/<component_id>"`.
- widgets: instantiation surface for ALL widgets in ONE call — prefer this over per-widget lookups
  when a dashboard uses more than one widget.
- widget: instantiation surface for one widget — the `widgetSelector` plus the `dynPath*`/`dynProp*`
  (camelCase) input pins `a2uiInstantiateWidget` exposes for its bound data paths and exposed props,
  and the action names usable for `fnRefs`."#,
        )
        .schema(json!({
            "type": "object",
            "properties": {
                "operation": { "type": "string", "enum": ["list", "page", "widgets", "widget"] },
                "app_id": { "type": "string", "description": "App id. Optional when FlowPilot knows the current app." },
                "board_id": { "type": "string", "description": "Restrict pages to this board. Optional." },
                "page_id": { "type": "string", "description": "Page id for operation 'page'." },
                "widget_selector": { "type": "string", "description": "Widget id or name for operation 'widget'." }
            }
        }));

    let handler: ToolHandler = Arc::new(move |_name, args| {
        frontend_tool_result(
            &bridge,
            "ui_inspect",
            args.clone(),
            FrontendToolApproval::none(),
        )
    });

    (tool, handler)
}

/// Catalog search tool - find nodes by functionality.
fn create_catalog_search_tool(provider: Option<Arc<dyn CatalogProvider>>) -> (Tool, ToolHandler) {
    let tool = Tool::new("catalog_search")
        .description(
            r#"Search the node catalog by functionality or name for read-only exploration and debugging.

WHEN TO USE: Explore catalog metadata when explaining a board or investigating a declaration issue.
FOR WORKFLOW EDITS: Use get_declarations for exact camelCase signatures, author the complete source
with write_flowscript, repair it with patch_flowscript, then check_flowscript and commit_flowscript.
FlowScript is the model-authored language; catalog_search is not part of that code-first lifecycle.
EXAMPLE QUERIES: "http request", "parse json", "loop array", "condition if", "open database""#,
        )
        .schema(json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural language catalog metadata search. For FlowScript authoring, use get_declarations instead."
                }
            },
            "required": ["query"]
        }));

    let handler: ToolHandler = Arc::new(move |_name, args| {
        let provider = provider.clone();
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let results: Vec<NodeMetadata> = if let Some(provider) = provider {
            block_on_tool(provider.search(&query))
        } else {
            Vec::new()
        };

        ToolResultObject::text(render_catalog_search_results(&results))
    });

    (tool, handler)
}

/// Turn a core rig tool definition into the Copilot SDK tool type, so both loops advertise
/// byte-identical name/description/schema.
fn tool_from_rig_definition<T: flow_like::flow::copilot::RigTool>(rig_tool: &T) -> Tool {
    let (name, description, parameters) = block_on_tool(tool_definition_parts(rig_tool));
    Tool::new(name).description(description).schema(parameters)
}

/// Get node details - full info about a specific node
fn create_get_node_details_tool(context: Arc<GraphContext>) -> (Tool, ToolHandler) {
    let tool = tool_from_rig_definition(&GetNodeDetailsTool {
        graph_context: context.clone(),
    });

    let handler: ToolHandler = Arc::new(move |_name, args| {
        let mut ids: Vec<String> = args
            .get("node_ids")
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|id| !id.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        if let Some(id) = args.get("node_id").and_then(Value::as_str) {
            let id = id.trim();
            if !id.is_empty() && !ids.iter().any(|existing| existing == id) {
                ids.insert(0, id.to_string());
            }
        }
        if ids.is_empty() {
            return ToolResultObject::text(
                "get_node_details needs `node_id` or a `node_ids` array.",
            );
        }
        let details = ids
            .iter()
            .map(|id| build_node_details_output(id, &context))
            .collect::<Vec<_>>()
            .join("\n\n");
        ToolResultObject::text(details)
    });

    (tool, handler)
}

/// Run the shared core emit validation (the exact checks the rig/Bits path runs) and flatten the
/// structured issues into model-facing strings: (errors, warnings). The visual-only model scope is
/// always enforced; graph/catalog checks additionally run when that context is available.
fn run_emit_validation(
    commands: &[BoardCommand],
    explanation: &str,
    graph_context: Option<&GraphContext>,
    provider: Option<&Arc<dyn CatalogProvider>>,
) -> (Vec<String>, Vec<String>, bool) {
    let args = EmitCommandsArgs {
        commands: commands.to_vec(),
        explanation: explanation.to_string(),
    };
    let scope = validate_model_facing_emit_commands_scope(&args);
    let outcome = if !scope.errors.is_empty() {
        scope
    } else if let (Some(graph_context), Some(provider)) = (graph_context, provider) {
        block_on_tool(validate_model_facing_emit_commands(
            &args,
            graph_context,
            provider.as_ref(),
        ))
    } else {
        scope
    };
    let requires_flowscript = emit_validation_requires_flowscript(&outcome);
    (
        outcome.errors.iter().map(format_validation_issue).collect(),
        outcome
            .warnings
            .iter()
            .map(format_validation_issue)
            .collect(),
        requires_flowscript,
    )
}

fn format_validation_issue(issue: &ValidationIssue) -> String {
    match issue.command_index {
        Some(index) => format!("[{}] command {}: {}", issue.code, index, issue.message),
        None => format!("[{}] {}", issue.code, issue.message),
    }
}

/// Emit commands tool - execute graph modifications. Validates internally: an invalid batch
/// queues nothing and reports the errors, so no separate validation round-trip is needed.
///
/// Queued commands are also pushed to `side_effect_commands` so transports that cannot parse
/// tool results (the external-agent MCP bridge for Claude Code/Codex) still surface them; the
/// SDK event loop extracts from tool results first and only drains the store as a fallback.
fn create_emit_commands_tool(
    graph_context: Option<Arc<GraphContext>>,
    provider: Option<Arc<dyn CatalogProvider>>,
    side_effect_commands: Option<Arc<Mutex<SideEffectCommandQueue>>>,
) -> (Tool, ToolHandler) {
    let tool = tool_from_rig_definition(&ModelFacingEmitCommandsTool);

    let handler: ToolHandler = Arc::new(move |_name, args| {
        let commands = args.get("commands").cloned().unwrap_or(json!([]));
        let explanation = args
            .get("explanation")
            .and_then(|v| v.as_str())
            .unwrap_or("Commands queued");

        // Parse commands from JSON
        let parsed_commands: Vec<BoardCommand> = match serde_json::from_value(commands.clone()) {
            Ok(cmds) => cmds,
            Err(e) => {
                return ToolResultObject::text(format!("Error parsing commands: {}", e));
            }
        };

        let (validation_errors, validation_warnings, requires_flowscript) = run_emit_validation(
            &parsed_commands,
            explanation,
            graph_context.as_deref(),
            provider.as_ref(),
        );
        if !validation_errors.is_empty() {
            // No command echo: the model already knows the batch it sent; the errors reference
            // command indices. Echoing the batch only bloats every retry's context.
            let result = json!({
                "status": if requires_flowscript { "representation_rejected" } else { "validation_errors" },
                "next_action": if requires_flowscript { "write_patch_check_commit_flowscript" } else { "repair_visual_batch" },
                "retry_emit_commands": !requires_flowscript,
                "errors": validation_errors,
                "warnings": validation_warnings,
                "explanation": explanation,
                "message": if requires_flowscript {
                    format!(
                        "Representation rejected, nothing was queued. Do not retry executable or layer commands through emit_commands. Author behavior with write_flowscript, repair with patch_flowscript, validate with check_flowscript, then queue with commit_flowscript:\n- {}",
                        validation_errors.join("\n- ")
                    )
                } else {
                    format!(
                        "Validation failed, nothing was queued. Fix these visual issues and call emit_commands again:\n- {}",
                        validation_errors.join("\n- ")
                    )
                }
            });
            return ToolResultObject::text(
                serde_json::to_string_pretty(&result).unwrap_or_default(),
            );
        }

        // Build summary
        let mut summary_lines: Vec<String> = Vec::new();
        summary_lines.push(format!("✓ Queued {} commands:", parsed_commands.len()));

        for cmd in &parsed_commands {
            let cmd_summary = match cmd {
                BoardCommand::AddNode {
                    node_type,
                    ref_id,
                    friendly_name,
                    ..
                } => {
                    format!(
                        "  - AddNode: {} (ref: {})",
                        friendly_name.as_deref().unwrap_or(node_type),
                        ref_id.as_deref().unwrap_or("none")
                    )
                }
                BoardCommand::AddPlaceholder { name, ref_id, .. } => {
                    format!(
                        "  - AddPlaceholder: \"{}\" (ref: {})",
                        name,
                        ref_id.as_deref().unwrap_or("none")
                    )
                }
                BoardCommand::ConnectPins {
                    from_node,
                    from_pin,
                    to_node,
                    to_pin,
                    ..
                } => {
                    format!(
                        "  - Connect: {}.{} → {}.{}",
                        from_node, from_pin, to_node, to_pin
                    )
                }
                BoardCommand::RemoveNode { node_id, .. } => {
                    format!("  - Remove node: {}", node_id)
                }
                BoardCommand::UpdateNodePin {
                    node_id, pin_id, ..
                } => {
                    format!("  - Update pin: {}.{}", node_id, pin_id)
                }
                _ => "  - Other command".to_string(),
            };
            summary_lines.push(cmd_summary);
        }

        summary_lines.push(format!("\nExplanation: {}", explanation));

        let queued_count = parsed_commands.len();
        if let Some(store) = &side_effect_commands
            && let Ok(mut queued) = store.lock()
        {
            if !queued.extend(parsed_commands) {
                return ToolResultObject::text(
                    json!({
                        "status": "error",
                        "code": "COMMAND_DELIVERY_CONFLICT",
                        "retryable": false,
                        "queued_count": 0,
                        "message": "This response already carries an exact retained workflow review. Direct commands were refused rather than mixing them under that review token; finish the existing Apply/Dismiss review."
                    })
                    .to_string(),
                );
            }
        }

        // The queued batch travels through the side-effect store (the chat loop drains it into a
        // <commands> frame); echoing it back to the model would only duplicate its own input.
        let result = json!({
            "status": "queued",
            "queued_count": queued_count,
            "explanation": explanation,
            "warnings": validation_warnings,
            "summary": summary_lines.join("\n")
        });

        ToolResultObject::text(serde_json::to_string_pretty(&result).unwrap_or_default())
    });

    (tool, handler)
}

/// get_declarations tool - look up FlowScript `.flow.d` signatures by intent.
fn create_get_declarations_tool(provider: Arc<dyn CatalogProvider>) -> (Tool, ToolHandler) {
    let tool = tool_from_rig_definition(&GetDeclarationsTool {
        provider: provider.clone(),
    });

    let handler: ToolHandler = Arc::new(move |_name, args| {
        let provider = provider.clone();
        let queries: Vec<String> = args
            .get("queries")
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|query| !query.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let declaration_args = GetDeclarationsArgs {
            query: args
                .get("query")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            queries,
        };
        let declarations = block_on_tool(run_declaration_queries(&provider, &declaration_args));
        ToolResultObject::text(declarations)
    });

    (tool, handler)
}

fn parse_flowscript_arguments<T: DeserializeOwned>(
    arguments: Value,
    tool_name: &str,
) -> Result<T, ToolResultObject> {
    serde_json::from_value(arguments).map_err(|error| {
        ToolResultObject::text(
            json!({
                "status": "validation_errors",
                "code": "FLOWSCRIPT_ARGUMENTS_INVALID",
                "message": format!(
                    "Failed to parse {tool_name} arguments against its advertised schema: {error}"
                )
            })
            .to_string(),
        )
    })
}

fn flowscript_tool_cancelled_result(
    tool_name: &str,
    draft_id: Option<&str>,
    revision: Option<u64>,
) -> ToolResultObject {
    ToolResultObject::text(
        serde_json::to_string_pretty(&json!({
            "status": "cancelled",
            "code": "FLOWSCRIPT_TOOL_CANCELLED",
            "draft_id": draft_id,
            "revision": revision,
            "message": format!(
                "{tool_name} was cancelled before a command batch was transferred. Continue from the retained FlowScript draft in the next phase."
            )
        }))
        .unwrap_or_default(),
    )
}

fn create_write_flowscript_tool(
    board: Arc<Board>,
    live_board: Option<Arc<AsyncMutex<Board>>>,
    provider: Arc<dyn CatalogProvider>,
    store: Arc<FlowIrDraftStore>,
    acceptance_binding: FlowIrAcceptanceBinding,
) -> (Tool, ToolHandler) {
    let tool = tool_from_rig_definition(&WriteFlowScriptTool {
        board: board.clone(),
        provider: provider.clone(),
        store: store.clone(),
        acceptance_binding: acceptance_binding.clone(),
    });
    let board_key = board.id.clone();
    let handler: ToolHandler = Arc::new(move |_name, arguments| {
        if super::frontend_tool_bridge::scoped_tool_execution_cancelled() {
            return flowscript_tool_cancelled_result("write_flowscript", None, None);
        }
        let store = match touch_bound_flowscript_draft_store(&board_key, store.clone()) {
            Ok(store) => store,
            Err(error) => return error.tool_result(),
        };
        let args = match parse_flowscript_arguments::<WriteFlowScriptArgs>(
            arguments.clone(),
            "write_flowscript",
        ) {
            Ok(args) => args,
            Err(error) => return error,
        };
        let catalog = block_on_tool(provider.get_all_metadata());
        let result = with_current_board(&board, live_board.as_ref(), |board| {
            store.observe_board(board);
            store.write_flowscript_with_acceptance_binding(
                board,
                &catalog,
                args,
                &acceptance_binding,
            )
        });
        schedule_flow_ir_draft_snapshot(&board_key, &store);
        ToolResultObject::text(serde_json::to_string_pretty(&result).unwrap_or_default())
    });
    (tool, handler)
}

fn create_patch_flowscript_tool(
    board: Arc<Board>,
    live_board: Option<Arc<AsyncMutex<Board>>>,
    provider: Arc<dyn CatalogProvider>,
    store: Arc<FlowIrDraftStore>,
    acceptance_binding: FlowIrAcceptanceBinding,
) -> (Tool, ToolHandler) {
    let tool = tool_from_rig_definition(&PatchFlowScriptTool {
        board: board.clone(),
        provider: provider.clone(),
        store: store.clone(),
        acceptance_binding: acceptance_binding.clone(),
    });
    let board_key = board.id.clone();
    let handler: ToolHandler = Arc::new(move |_name, arguments| {
        if super::frontend_tool_bridge::scoped_tool_execution_cancelled() {
            return flowscript_tool_cancelled_result("patch_flowscript", None, None);
        }
        let store = match touch_bound_flowscript_draft_store(&board_key, store.clone()) {
            Ok(store) => store,
            Err(error) => return error.tool_result(),
        };
        let args = match parse_flowscript_arguments::<PatchFlowScriptArgs>(
            arguments.clone(),
            "patch_flowscript",
        ) {
            Ok(args) => args,
            Err(error) => return error,
        };
        let catalog = block_on_tool(provider.get_all_metadata());
        let result = with_current_board(&board, live_board.as_ref(), |board| {
            store.observe_board(board);
            store.patch_flowscript_with_acceptance_binding(
                board,
                &catalog,
                args,
                &acceptance_binding,
            )
        });
        schedule_flow_ir_draft_snapshot(&board_key, &store);
        ToolResultObject::text(serde_json::to_string_pretty(&result).unwrap_or_default())
    });
    (tool, handler)
}

fn create_check_flowscript_tool(
    board: Arc<Board>,
    live_board: Option<Arc<AsyncMutex<Board>>>,
    provider: Arc<dyn CatalogProvider>,
    store: Arc<FlowIrDraftStore>,
    acceptance_binding: FlowIrAcceptanceBinding,
) -> (Tool, ToolHandler) {
    let tool = tool_from_rig_definition(&CheckFlowScriptTool {
        board: board.clone(),
        provider: provider.clone(),
        store: store.clone(),
        acceptance_binding: acceptance_binding.clone(),
    });
    let board_key = board.id.clone();
    let handler: ToolHandler = Arc::new(move |_name, arguments| {
        if super::frontend_tool_bridge::scoped_tool_execution_cancelled() {
            return flowscript_tool_cancelled_result("check_flowscript", None, None);
        }
        let store = match touch_bound_flowscript_draft_store(&board_key, store.clone()) {
            Ok(store) => store,
            Err(error) => return error.tool_result(),
        };
        let args = match parse_flowscript_arguments::<CheckFlowScriptArgs>(
            arguments.clone(),
            "check_flowscript",
        ) {
            Ok(args) => args,
            Err(error) => return error,
        };
        let catalog = block_on_tool(provider.get_all_metadata());
        let result = with_current_board(&board, live_board.as_ref(), |board| {
            store.observe_board(board);
            store.check_flowscript_with_acceptance_binding(
                board,
                &catalog,
                args,
                &acceptance_binding,
            )
        });
        schedule_flow_ir_draft_snapshot(&board_key, &store);
        ToolResultObject::text(serde_json::to_string_pretty(&result).unwrap_or_default())
    });
    (tool, handler)
}

fn create_commit_flowscript_tool(
    board: Arc<Board>,
    live_board: Option<Arc<AsyncMutex<Board>>>,
    provider: Arc<dyn CatalogProvider>,
    store: Arc<FlowIrDraftStore>,
    acceptance_binding: FlowIrAcceptanceBinding,
    side_effect_commands: Option<Arc<Mutex<SideEffectCommandQueue>>>,
    queued_flowscript: Option<Arc<Mutex<Option<String>>>>,
) -> (Tool, ToolHandler) {
    let tool = tool_from_rig_definition(&CommitFlowScriptTool {
        board: board.clone(),
        provider: provider.clone(),
        store: store.clone(),
        acceptance_binding: acceptance_binding.clone(),
    });
    let board_key = board.id.clone();
    let handler: ToolHandler = Arc::new(move |_name, arguments| {
        let store = match touch_bound_flowscript_draft_store(&board_key, store.clone()) {
            Ok(store) => store,
            Err(error) => return error.tool_result(),
        };
        let args = match parse_flowscript_arguments::<CommitFlowScriptArgs>(
            arguments.clone(),
            "commit_flowscript",
        ) {
            Ok(args) => args,
            Err(error) => return error,
        };
        let draft_id = args.draft_id.clone();
        let expected_revision = args.expected_revision;
        if super::frontend_tool_bridge::scoped_tool_execution_cancelled() {
            return flowscript_tool_cancelled_result(
                "commit_flowscript",
                Some(&draft_id),
                Some(expected_revision),
            );
        }
        let catalog = block_on_tool(provider.get_all_metadata());

        let tool_result = with_current_board(&board, live_board.as_ref(), |board| {
            // Keep the registry-backed board guard across fingerprint validation and host queue
            // installation. The client never supplies a command batch: only commands retained by
            // check_flowscript for this exact revision cross the Apply/Dismiss boundary.
            store.observe_board(board);
            let result = store.commit_flowscript_with_acceptance_binding(
                board,
                &catalog,
                args,
                &acceptance_binding,
            );
            let commit_token = if result.status == "queued" {
                store
                    .latest_pending_commit_token(&board.id)
                    .filter(|token| {
                        token.draft_id == draft_id
                            && token.revision == expected_revision
                            && result.base_fingerprint.as_deref()
                                == Some(token.base_fingerprint.as_str())
                    })
            } else {
                None
            };

            if result.status == "queued" && commit_token.is_none() {
                let released = store.release_commit(&draft_id, expected_revision);
                return ToolResultObject::text(
                    json!({
                        "status": "error",
                        "code": "FLOWSCRIPT_COMMIT_TOKEN_INVALID",
                        "draft_id": draft_id,
                        "revision": expected_revision,
                        "claim_released": released,
                        "source": result.source,
                        "message": "FlowScript committed without a complete board/revision/claim identity. No commands were transferred; the malformed pre-delivery claim was rolled back."
                    })
                    .to_string(),
                );
            }

            if super::frontend_tool_bridge::scoped_tool_execution_cancelled() {
                if let Some(token) = commit_token.as_ref() {
                    release_retained_commit_claim(&store, token);
                }
                return flowscript_tool_cancelled_result(
                    "commit_flowscript",
                    Some(&draft_id),
                    Some(expected_revision),
                );
            }

            if let Some(commit_token) = commit_token {
                let Some(commands) = &side_effect_commands else {
                    release_retained_commit_claim(&store, &commit_token);
                    return ToolResultObject::text(
                        json!({
                            "status": "error",
                            "code": "FLOWSCRIPT_COMMIT_QUEUE_UNAVAILABLE",
                            "draft_id": draft_id,
                            "revision": expected_revision,
                            "source": result.source,
                            "message": "FlowScript checked successfully, but the host command queue is unavailable. The claim was released; retry this exact revision when the queue is available."
                        })
                        .to_string(),
                    );
                };
                if result.commands.is_empty() || result.commands.len() != result.queued_count {
                    release_retained_commit_claim(&store, &commit_token);
                    return ToolResultObject::text(
                        json!({
                            "status": "error",
                            "code": "FLOWSCRIPT_COMMIT_BATCH_INVALID",
                            "draft_id": draft_id,
                            "revision": expected_revision,
                            "source": result.source,
                            "message": "The retained FlowScript command batch was incomplete. No commands were transferred and the claim was released."
                        })
                        .to_string(),
                    );
                }
                let mut queued = match commands.lock() {
                    Ok(queued) => queued,
                    Err(poisoned) => {
                        poisoned.into_inner().abandon();
                        release_retained_commit_claim(&store, &commit_token);
                        return ToolResultObject::text(
                            json!({
                                "status": "error",
                                "code": "FLOWSCRIPT_COMMIT_QUEUE_UNAVAILABLE",
                                "draft_id": draft_id,
                                "revision": expected_revision,
                                "source": result.source,
                                "message": "The host command queue could not be locked. The FlowScript claim was released; retry this exact revision."
                            })
                            .to_string(),
                        );
                    }
                };
                if super::frontend_tool_bridge::scoped_tool_execution_cancelled() {
                    release_retained_commit_claim(&store, &commit_token);
                    return flowscript_tool_cancelled_result(
                        "commit_flowscript",
                        Some(&draft_id),
                        Some(expected_revision),
                    );
                }
                let source = result.source.clone();
                if !queued.extend_retained_commit(
                    result.commands.clone(),
                    store.clone(),
                    commit_token,
                    queued_flowscript.clone(),
                    source.clone(),
                ) {
                    return ToolResultObject::text(
                        json!({
                            "status": "error",
                            "code": "FLOWSCRIPT_COMMIT_TOKEN_CONFLICT",
                            "draft_id": draft_id,
                            "revision": expected_revision,
                            "source": source,
                            "message": "This FlowPilot response already carries unresolved commands or another commit token. The newer FlowScript claim was released rather than mixing batches under one review token."
                        })
                        .to_string(),
                    );
                }
                if let (Some(workspace), Some(source)) = (&queued_flowscript, source)
                    && let Ok(mut queued_workspace) = workspace.lock()
                {
                    *queued_workspace = Some(source);
                }
            }

            // FlowScriptDraftResponse skips its host-only commands field, preventing a second,
            // client-trusted copy of the batch from escaping through the model tool result.
            ToolResultObject::text(serde_json::to_string_pretty(&result).unwrap_or_default())
        });
        schedule_flow_ir_draft_snapshot(&board_key, &store);
        tool_result
    });
    (tool, handler)
}

#[allow(dead_code)]
fn create_plan_flow_ir_tool(provider: Arc<dyn CatalogProvider>) -> (Tool, ToolHandler) {
    let tool = tool_from_rig_definition(&PlanFlowIrTool {
        provider: provider.clone(),
    });
    let handler: ToolHandler = Arc::new(move |_name, args| {
        let request = match parse_typed_ir_arguments::<FlowCapabilityPlanRequest>(
            args.clone(),
            "IR_CAPABILITY_PLAN_INVALID",
            "typed capability plan",
        ) {
            Ok(request) => request,
            Err(error) => return ToolResultObject::text(error),
        };
        let catalog = block_on_tool(provider.get_all_metadata());
        ToolResultObject::text(
            serde_json::to_string_pretty(&plan_flow_capabilities(&request, &catalog))
                .unwrap_or_default(),
        )
    });
    (tool, handler)
}

#[allow(dead_code)]
fn create_begin_flow_ir_draft_tool(
    board: Arc<Board>,
    live_board: Option<Arc<AsyncMutex<Board>>>,
    provider: Arc<dyn CatalogProvider>,
    store: Arc<FlowIrDraftStore>,
    acceptance_binding: Option<FlowIrAcceptanceBinding>,
) -> (Tool, ToolHandler) {
    let tool = match acceptance_binding.as_ref() {
        Some(binding) => tool_from_rig_definition(&BoundBeginFlowIrDraftTool {
            board: board.clone(),
            provider: provider.clone(),
            store: store.clone(),
            acceptance_binding: binding.clone(),
        }),
        None => tool_from_rig_definition(&BeginFlowIrDraftTool {
            board: board.clone(),
            provider: provider.clone(),
            store: store.clone(),
        }),
    };
    let board_key = board.id.clone();
    let handler: ToolHandler = Arc::new(move |_name, args| {
        let store = match touch_persisted_flow_ir_draft_store(&board_key, store.clone(), None) {
            Ok(store) => store,
            Err(error) => return error.tool_result(),
        };
        let args = match parse_typed_ir_arguments::<BeginFlowIrDraftArgs>(
            args.clone(),
            "IR_DRAFT_INVALID",
            "typed draft header",
        ) {
            Ok(args) => args,
            Err(error) => return ToolResultObject::text(error),
        };
        let catalog = block_on_tool(provider.get_all_metadata());
        ToolResultObject::text(
            serde_json::to_string_pretty(&with_current_board(
                &board,
                live_board.as_ref(),
                |board| {
                    store.observe_board(board);
                    match acceptance_binding.as_ref() {
                        Some(binding) => {
                            store.begin_with_acceptance_binding(board, &catalog, args, binding)
                        }
                        None => store.begin(board, &catalog, args),
                    }
                },
            ))
            .unwrap_or_default(),
        )
    });
    (tool, handler)
}

#[allow(dead_code)]
fn create_update_flow_ir_draft_tool(
    board: Arc<Board>,
    live_board: Option<Arc<AsyncMutex<Board>>>,
    provider: Arc<dyn CatalogProvider>,
    store: Arc<FlowIrDraftStore>,
    acceptance_binding: Option<FlowIrAcceptanceBinding>,
) -> (Tool, ToolHandler) {
    let tool = tool_from_rig_definition(&UpdateFlowIrDraftTool {
        board: board.clone(),
        provider: provider.clone(),
        store: store.clone(),
    });
    let board_key = board.id.clone();
    let handler: ToolHandler = Arc::new(move |_name, args| {
        let store = match touch_persisted_flow_ir_draft_store(&board_key, store.clone(), None) {
            Ok(store) => store,
            Err(error) => return error.tool_result(),
        };
        let args = match parse_typed_ir_arguments::<UpdateFlowIrDraftArgs>(
            args.clone(),
            "IR_DRAFT_UPDATE_INVALID",
            "typed draft update",
        ) {
            Ok(args) => args,
            Err(error) => return ToolResultObject::text(error),
        };
        if let Some(denied) = typed_draft_request_access_denied(
            &store,
            &board_key,
            &args.draft_id,
            acceptance_binding.as_ref(),
        ) {
            return denied;
        }
        let catalog = block_on_tool(provider.get_all_metadata());
        ToolResultObject::text(
            serde_json::to_string_pretty(&with_current_board(
                &board,
                live_board.as_ref(),
                |board| {
                    store.observe_board(board);
                    match acceptance_binding.as_ref() {
                        Some(binding) => store
                            .update_draft_with_acceptance_binding(board, &catalog, args, binding),
                        None => store.update_draft(board, &catalog, args),
                    }
                },
            ))
            .unwrap_or_default(),
        )
    });
    (tool, handler)
}

#[allow(dead_code)]
fn create_upsert_flow_ir_module_tool(
    board: Arc<Board>,
    live_board: Option<Arc<AsyncMutex<Board>>>,
    provider: Arc<dyn CatalogProvider>,
    store: Arc<FlowIrDraftStore>,
    acceptance_binding: Option<FlowIrAcceptanceBinding>,
) -> (Tool, ToolHandler) {
    let tool = tool_from_rig_definition(&UpsertFlowIrModuleTool {
        board: board.clone(),
        provider: provider.clone(),
        store: store.clone(),
    });
    let board_key = board.id.clone();
    let handler: ToolHandler = Arc::new(move |_name, args| {
        let store = match touch_persisted_flow_ir_draft_store(&board_key, store.clone(), None) {
            Ok(store) => store,
            Err(error) => return error.tool_result(),
        };
        let args = match parse_typed_ir_arguments::<UpsertFlowIrModuleArgs>(
            args.clone(),
            "IR_MODULE_INVALID",
            "typed workflow module",
        ) {
            Ok(args) => args,
            Err(error) => return ToolResultObject::text(error),
        };
        if let Some(denied) = typed_draft_request_access_denied(
            &store,
            &board_key,
            &args.draft_id,
            acceptance_binding.as_ref(),
        ) {
            return denied;
        }
        let catalog = block_on_tool(provider.get_all_metadata());
        ToolResultObject::text(
            serde_json::to_string_pretty(&with_current_board(
                &board,
                live_board.as_ref(),
                |board| {
                    store.observe_board(board);
                    match acceptance_binding.as_ref() {
                        Some(binding) => store
                            .upsert_module_with_acceptance_binding(board, &catalog, args, binding),
                        None => store.upsert_module(board, &catalog, args),
                    }
                },
            ))
            .unwrap_or_default(),
        )
    });
    (tool, handler)
}

#[allow(dead_code)]
fn create_validate_flow_ir_draft_tool(
    board: Arc<Board>,
    live_board: Option<Arc<AsyncMutex<Board>>>,
    provider: Arc<dyn CatalogProvider>,
    store: Arc<FlowIrDraftStore>,
    acceptance_binding: Option<FlowIrAcceptanceBinding>,
) -> (Tool, ToolHandler) {
    let tool = tool_from_rig_definition(&ValidateFlowIrDraftTool {
        board: board.clone(),
        provider: provider.clone(),
        store: store.clone(),
    });
    let board_key = board.id.clone();
    let handler: ToolHandler = Arc::new(move |_name, args| {
        let store = match touch_persisted_flow_ir_draft_store(&board_key, store.clone(), None) {
            Ok(store) => store,
            Err(error) => return error.tool_result(),
        };
        let args = match parse_typed_ir_arguments::<ValidateFlowIrDraftArgs>(
            args.clone(),
            "IR_DRAFT_VALIDATION_INVALID",
            "typed draft validation request",
        ) {
            Ok(args) => args,
            Err(error) => return ToolResultObject::text(error),
        };
        if let Some(denied) = typed_draft_request_access_denied(
            &store,
            &board_key,
            &args.draft_id,
            acceptance_binding.as_ref(),
        ) {
            return denied;
        }
        let catalog = block_on_tool(provider.get_all_metadata());
        ToolResultObject::text(
            serde_json::to_string_pretty(&with_current_board(
                &board,
                live_board.as_ref(),
                |board| {
                    store.observe_board(board);
                    match acceptance_binding.as_ref() {
                        Some(binding) => {
                            store.validate_with_acceptance_binding(board, &catalog, args, binding)
                        }
                        None => store.validate(board, &catalog, args),
                    }
                },
            ))
            .unwrap_or_default(),
        )
    });
    (tool, handler)
}

#[allow(dead_code)]
fn create_commit_flow_ir_draft_tool(
    board: Arc<Board>,
    live_board: Option<Arc<AsyncMutex<Board>>>,
    provider: Arc<dyn CatalogProvider>,
    store: Arc<FlowIrDraftStore>,
    acceptance_binding: Option<FlowIrAcceptanceBinding>,
    side_effect_commands: Option<Arc<Mutex<SideEffectCommandQueue>>>,
    queued_flowscript: Option<Arc<Mutex<Option<String>>>>,
) -> (Tool, ToolHandler) {
    let tool = tool_from_rig_definition(&CommitFlowIrDraftTool {
        board: board.clone(),
        provider: provider.clone(),
        store: store.clone(),
    });
    let board_key = board.id.clone();
    let handler: ToolHandler = Arc::new(move |_name, args| {
        let store = match touch_persisted_flow_ir_draft_store(&board_key, store.clone(), None) {
            Ok(store) => store,
            Err(error) => return error.tool_result(),
        };
        let args = match parse_typed_ir_arguments::<CommitFlowIrDraftArgs>(
            args.clone(),
            "IR_COMMIT_INVALID",
            "typed draft commit",
        ) {
            Ok(args) => args,
            Err(error) => return ToolResultObject::text(error),
        };
        if let Some(denied) = typed_draft_request_access_denied(
            &store,
            &board_key,
            &args.draft_id,
            acceptance_binding.as_ref(),
        ) {
            return denied;
        }
        let draft_id = args.draft_id.clone();
        let expected_revision = args.expected_revision;
        let cancelled_result = || {
            ToolResultObject::text(
                serde_json::to_string_pretty(&json!({
                    "status": "cancelled",
                    "code": "IR_COMMIT_CANCELLED",
                    "draft_id": draft_id.clone(),
                    "revision": expected_revision,
                    "message": "Typed draft commit was cancelled before queueing commands."
                }))
                .unwrap_or_default(),
            )
        };
        if super::frontend_tool_bridge::scoped_tool_execution_cancelled() {
            return cancelled_result();
        }
        let catalog = block_on_tool(provider.get_all_metadata());
        if super::frontend_tool_bridge::scoped_tool_execution_cancelled() {
            return cancelled_result();
        }
        with_current_board(&board, live_board.as_ref(), |board| {
            // The registry-backed board guard remains held until the command batch and its claim
            // are atomically installed in the host queue. This closes the captured-board race at
            // the fingerprint/queue boundary.
            store.observe_board(board);
            let result = match acceptance_binding.as_ref() {
                Some(binding) => {
                    store.commit_with_acceptance_binding(board, &catalog, args, binding)
                }
                None => store.commit(board, &catalog, args),
            };
            let commit_token = if result.status == "queued" {
                match (
                    result.base_fingerprint.clone(),
                    result.claim_id.clone(),
                    result.revision,
                ) {
                    (Some(base_fingerprint), Some(claim_id), Some(revision))
                        if revision == expected_revision
                            && !board.id.trim().is_empty()
                            && !claim_id.trim().is_empty() =>
                    {
                        Some(FlowIrCommitToken {
                            board_id: board.id.clone(),
                            draft_id: draft_id.clone(),
                            revision,
                            requires_destructive_approval: store
                                .pending_commit_requires_destructive_approval(
                                    &draft_id,
                                    revision,
                                    &base_fingerprint,
                                    &claim_id,
                                )
                                .unwrap_or(true),
                            base_fingerprint,
                            claim_id,
                        })
                    }
                    _ => {
                        // Core promises a complete nonce for every newly queued commit. If that
                        // invariant is ever broken, no exact token exists to carry through the
                        // normal lifecycle. Roll back synchronously before exposing commands; this
                        // is the only legacy revision-only release path and cannot race a retry
                        // because the malformed claim is still pending at this point.
                        let released = store.release_commit(&draft_id, expected_revision);
                        return ToolResultObject::text(
                            json!({
                                "status": "error",
                                "code": "IR_COMMIT_TOKEN_INVALID",
                                "draft_id": draft_id.clone(),
                                "revision": expected_revision,
                                "claim_released": released,
                                "message": "Typed draft queued without a complete board/revision/claim identity. No commands were transferred; the malformed pre-delivery claim was rolled back."
                            })
                            .to_string(),
                        );
                    }
                }
            } else {
                None
            };
            if super::frontend_tool_bridge::scoped_tool_execution_cancelled() {
                if let Some(token) = commit_token.as_ref() {
                    release_retained_commit_claim(&store, token);
                }
                return cancelled_result();
            }
            if let Some(commit_token) = commit_token {
                let Some(commands) = &side_effect_commands else {
                    release_retained_commit_claim(&store, &commit_token);
                    return ToolResultObject::text(
                        json!({
                            "status": "error",
                            "code": "IR_COMMIT_QUEUE_UNAVAILABLE",
                            "draft_id": draft_id.clone(),
                            "revision": expected_revision,
                            "message": "Typed draft validated, but the host command queue is unavailable. The commit claim was released; retry this exact revision when the queue is available."
                        })
                        .to_string(),
                    );
                };
                if super::frontend_tool_bridge::scoped_tool_execution_cancelled() {
                    release_retained_commit_claim(&store, &commit_token);
                    return cancelled_result();
                }
                let mut queued = match commands.lock() {
                    Ok(queued) => queued,
                    Err(poisoned) => {
                        // A poisoned queue cannot prove delivery. Reopen all pending typed claims
                        // and discard their command batches before returning a retryable host error.
                        poisoned.into_inner().abandon();
                        release_retained_commit_claim(&store, &commit_token);
                        return ToolResultObject::text(
                            json!({
                                "status": "error",
                                "code": "IR_COMMIT_QUEUE_UNAVAILABLE",
                                "draft_id": draft_id.clone(),
                                "revision": expected_revision,
                                "message": "Typed draft validated, but the host command queue could not be locked. The commit claim was released; retry this exact revision."
                            })
                            .to_string(),
                        );
                    }
                };
                if super::frontend_tool_bridge::scoped_tool_execution_cancelled() {
                    release_retained_commit_claim(&store, &commit_token);
                    return cancelled_result();
                }
                if !queued.extend_retained_commit(
                    result.commands.clone(),
                    store.clone(),
                    commit_token,
                    queued_flowscript.clone(),
                    result.flowscript.clone(),
                ) {
                    return ToolResultObject::text(
                        json!({
                            "status": "error",
                            "code": "IR_COMMIT_TOKEN_CONFLICT",
                            "draft_id": draft_id.clone(),
                            "revision": expected_revision,
                            "message": "This FlowPilot response already carries unresolved commands or another typed commit token. The newer claim was released rather than mixing batches under one review token."
                        })
                        .to_string(),
                    );
                }
                if let (Some(workspace), Some(flowscript)) =
                    (&queued_flowscript, result.flowscript.clone())
                    && let Ok(mut queued_workspace) = workspace.lock()
                {
                    *queued_workspace = Some(flowscript);
                }
            }
            ToolResultObject::text(serde_json::to_string_pretty(&result).unwrap_or_default())
        })
    });
    (tool, handler)
}

/// Render the current live board for a retained FlowScript source lifecycle. The handler uses the
/// registry-backed board lock so a long-running agent never starts from the stale construction-time
/// snapshot used only to derive the shared Rig schema.
fn create_get_current_flowscript_tool(
    board: Arc<Board>,
    live_board: Option<Arc<AsyncMutex<Board>>>,
) -> (Tool, ToolHandler) {
    let tool = tool_from_rig_definition(&GetCurrentFlowScriptTool {
        board: board.clone(),
    });

    let handler: ToolHandler = Arc::new(move |_name, _args| {
        let flowscript = with_current_board(&board, live_board.as_ref(), |board| {
            board_to_flowscript(
                board,
                &RenderOptions {
                    anchors: true,
                    ..Default::default()
                },
            )
        });
        let payload = json!({
            "status": "ok",
            "source": flowscript,
            "message": "Use this exact anchored FlowScript as the starting source for write_flowscript. Repair the retained document with patch_flowscript, run check_flowscript on its exact revision, then call commit_flowscript once it is valid."
        });
        ToolResultObject::text(serde_json::to_string_pretty(&payload).unwrap_or_default())
    });

    (tool, handler)
}

#[allow(dead_code)]
fn create_edit_flowscript_tool(
    board: Arc<Board>,
    provider: Option<Arc<dyn CatalogProvider>>,
    side_effect_commands: Option<Arc<Mutex<SideEffectCommandQueue>>>,
    queued_flowscript: Option<Arc<Mutex<Option<String>>>>,
) -> (Tool, ToolHandler) {
    let tool = Tool::new("edit_flowscript")
        .description(
            r#"Legacy compatibility adapter for applying one complete edited FlowScript document.
New model-facing surfaces use write_flowscript, patch_flowscript, check_flowscript, and
commit_flowscript so source revisions and exact compiler-derived commands remain retained.

For existing-board edits, call `get_current_flowscript` first, edit that exact returned document,
and submit the FULL edited FlowScript source. Reconcile compares it to the live board using the
`//@n:<id>` anchor comments and catalog declarations, then produces minimal changes:
- A changed literal argument on an anchored call → updates that node's pin value.
- An anchored statement you removed → deletes that node only when `allow_deletions` is true.
- A new unanchored FlowScript call → adds that node, configures literal args, and connects
  resolvable FlowScript references/nested calls.
- A new unanchored `function name(...) { ... }` declaration → creates a Function layer, places
  body nodes inside it, creates boundary pins from params/returns, and wires `return` values.

VALIDATION: This tool validates before queueing. If it reports parse errors or diagnostics,
nothing was queued — revise the SAME submitted draft and call edit_flowscript again immediately.
Do not restart broad catalog discovery; make one targeted declaration lookup only when a diagnostic
explicitly identifies a missing/incorrect declaration. Only a clean parse queues commands. After
status `queued`, stop: do not search or submit the document again.

COMPLETENESS: An Event entry is added only after the board logic is complete. Never replace a
failed full draft with an empty `eventsSimple() {}`/`eventsGeneric() {}`/`eventsChat() {}` shell;
that entry is only the registration target for the outer app Event and is not a workflow by itself.
For complex flows, keep real work in focused, non-empty named helper functions and add thin Event
entries last to invoke those helpers. Across validation retries preserve the requested helpers,
Events, variables, and capabilities. A direct one-node log/string-format smoke test is not a valid
replacement for a richer production draft, even if that smaller document parses cleanly.

RULES:
- PRESERVE every `//@n:<id>` anchor comment on statements you keep, exactly as given.
- Leave `allow_deletions` false unless the user explicitly asked to delete existing board items.
- Do NOT invent anchors for brand-new nodes; write normal unanchored calls using declarations
  from `get_declarations`.
- If you use `variableGet({ varRef: "NAME" })` or any `varRef`, `NAME` must resolve to an
  existing variable or a top-level FlowScript variable declaration such as
  `const NAME: string = ""`; missing varRefs are validation errors.
- FlowScript statement order maps to the normal execution path only when the previous node has one
  execution output, a `done` / `exec_done` output, or an explicit continuation policy in the
  reconciler. Multi-output nodes are not guessed by pin order; API Call/httpFetch continues from
  `exec_success`, never `exec_error`. If no policy exists, validation reports a diagnostic instead
  of queueing an unsafe edge.
- Existing multi-output execution graphs render back to FlowScript as labelled branch blocks, so
  board -> FlowScript -> board preserves those branches rather than flattening them.
- Streaming calls with `on_stream` plus `exec_done` may place `.chunk` consumers immediately after
  the call; those consumers wire from `on_stream`, while later `.response` / `.stats` consumers
  continue from `exec_done`.
- For loops, the body is the `exec_out` path and the next statement continues from `done` /
  `exec_done`; make sure the loop's `array` input receives the array being iterated.
- Helper `function` declarations are fully supported: calling `helperName(args)` creates a Call
  Function node wired to that function's layer, impure bodies chain from the layer's `exec_in`
  boundary pin, and `return` values surface as call-node outputs. USE THEM — a single layer
  (root, event scope, or one function) is hard-capped at 50 nodes and edits exceeding it are
  rejected, so split big flows into small helper functions with focused responsibilities.
- The `function` keyword is mandatory for helpers: write
  `function fetchMail(host: string) { ... }`, never bare `fetchMail(...) { ... }`. A helper call is
  valid only when its declaration remains in this same full document; do not invent helper calls
  and expect declaration lookup to resolve them as catalog nodes.
- A helper that executes `return value` must declare its named return pin, for example
  `function classify(body: string): (isSupport: bool) { ...; return result.value }`; otherwise the
  Function layer has no output pin for that return.
- Charts (`a2uiPushCsvToChart`) read their data from a `format`-specific pin. With `format: "CSV"`, wire
  a DataFusion query's `table` output into the chart's `table` input (both are the same tabular struct)
  and set `chartType` (for example "Bar" / "Line" / "Pie"). The `data` input is ONLY for
  `format: "JSON"`. Wiring a `table` output into `data` with `format: "CSV"` leaves the chart's data
  unset and fails at run time.
- Read a struct field with `structGet({ struct: <structValue>, field: "name" }).value` (its `value`
  output is the field). To target an a2ui element, either pass the element id path string directly to a
  setter's `elementRef`, or fetch a handle with `a2uiGetElement({ elementRef: "surfaceId/element-id" }).element`;
  both are accepted.
- To reposition nodes on the canvas without changing layer membership, use `emit_commands` with MoveNode."#,
        )
        .schema(json!({
            "type": "object",
            "properties": {
                "flowscript": {
                    "type": "string",
                    "description": "The full edited FlowScript source for the board, with anchors preserved."
                },
                "allow_deletions": {
                    "type": "boolean",
                    "description": "Set true only when the user explicitly requested deletion of existing board items. Defaults false to prevent incomplete FlowScript from deleting nodes."
                }
            },
            "required": ["flowscript"]
        }));

    let handler: ToolHandler = Arc::new(move |_name, args| {
        let flowscript = args
            .get("flowscript")
            .or_else(|| args.get("script"))
            .or_else(|| args.get("source"))
            .or_else(|| args.get("content"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let allow_deletions = args
            .get("allow_deletions")
            .or_else(|| args.get("allowDeletions"))
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let cancelled_result = || {
            let payload = json!({
                "status": "cancelled",
                "retryable": true,
                "next_action": "resume_retained_draft",
                "flowscript_workspace_summary": flowscript_summary(flowscript),
                "message": "The owning agent phase ended while this FlowScript was being validated. No commands were queued by this cancelled call; continue from the retained full draft in the next phase."
            });
            ToolResultObject::text(serde_json::to_string_pretty(&payload).unwrap_or_default())
        };

        if super::frontend_tool_bridge::scoped_tool_execution_cancelled() {
            return cancelled_result();
        }

        if flowscript.trim().is_empty() {
            let payload = json!({
                "status": "validation_errors",
                "retryable": true,
                "next_action": "revise_and_resubmit",
                "errors": ["edit_flowscript requires a non-empty `flowscript` string. The submitted tool arguments did not contain usable FlowScript."],
                "flowscript_workspace_summary": flowscript_summary(flowscript),
                "message": "FlowScript validation failed. Call edit_flowscript again with the edited FlowScript in `flowscript`."
            });
            return ToolResultObject::text(
                serde_json::to_string_pretty(&payload).unwrap_or_default(),
            );
        }

        if board_has_no_nodes(&board) && !flowscript_has_executable_node_call(flowscript) {
            let payload = json!({
                "status": "validation_errors",
                "retryable": true,
                "next_action": "revise_and_resubmit",
                "errors": ["An empty Event entry is only a registration target, not a workflow implementation."],
                "flowscript_workspace_summary": flowscript_summary(flowscript),
                "message": "Nothing was queued. Restore the complete prior draft, implement the board logic first, and keep the Event entry as the final execution root."
            });
            return ToolResultObject::text(
                serde_json::to_string_pretty(&payload).unwrap_or_default(),
            );
        }

        let catalog = provider
            .clone()
            .map(|provider| block_on_tool(provider.get_all_metadata()))
            .unwrap_or_default();

        if super::frontend_tool_bridge::scoped_tool_execution_cancelled() {
            return cancelled_result();
        }

        let result = reconcile_text_with_catalog(&board, flowscript, &catalog);
        let structured_diagnostics = result.structured_diagnostics_for_source(flowscript);
        if super::frontend_tool_bridge::scoped_tool_execution_cancelled() {
            return cancelled_result();
        }
        let has_parse_error = result
            .diagnostics
            .iter()
            .any(|d| d.to_lowercase().contains("parse error"));
        let blocking_diagnostics = result
            .diagnostics
            .iter()
            .filter(|diagnostic| is_blocking_flowscript_diagnostic(diagnostic))
            .cloned()
            .collect::<Vec<_>>();

        // Parse failure, unsafe partial translation, or no derivable change with diagnostics →
        // report back and queue nothing. This matches the core/Bits path: partial commands must
        // never turn a semantically incomplete FlowScript into a green success.
        if has_parse_error
            || !blocking_diagnostics.is_empty()
            || (result.commands.is_empty() && !result.diagnostics.is_empty())
        {
            let message = flowscript_validation_message(flowscript, &result.diagnostics);
            let payload = json!({
                "status": "validation_errors",
                "retryable": true,
                "next_action": "revise_and_resubmit",
                "errors": if blocking_diagnostics.is_empty() { result.diagnostics.clone() } else { blocking_diagnostics },
                "diagnostics": result.diagnostics,
                "structured_diagnostics": structured_diagnostics,
                "flowscript_workspace_summary": flowscript_summary(flowscript),
                "message": format!("{message} Revise this same draft and call edit_flowscript again; do not restart broad discovery.")
            });
            return ToolResultObject::text(
                serde_json::to_string_pretty(&payload).unwrap_or_default(),
            );
        }

        // Clean parse but no changes derived → nothing to do.
        if result.commands.is_empty() {
            let payload = json!({
                "status": "no_changes",
                "retryable": true,
                "next_action": "revise_and_resubmit",
                "flowscript_workspace_summary": flowscript_summary(flowscript),
                "message": "No board changes were derived from the FlowScript. Revise this same draft with concrete catalog calls inside a function/event block and call edit_flowscript again. Use at most one targeted declaration lookup if an exact function is missing; do not restart broad discovery."
            });
            return ToolResultObject::text(
                serde_json::to_string_pretty(&payload).unwrap_or_default(),
            );
        }

        if !allow_deletions {
            let destructive = destructive_flowscript_command_summaries(&result.commands);
            if !destructive.is_empty() {
                let message = blocked_destructive_flowscript_message(&destructive);
                let payload = json!({
                    "status": "validation_errors",
                    "retryable": true,
                    "next_action": "revise_and_resubmit",
                    "errors": [message],
                    "diagnostics": result.diagnostics,
                    "structured_diagnostics": structured_diagnostics,
                    "flowscript_workspace_summary": flowscript_summary(flowscript),
                    "message": "FlowScript validation failed. Deletions require an explicit allow_deletions=true opt-in."
                });
                return ToolResultObject::text(
                    serde_json::to_string_pretty(&payload).unwrap_or_default(),
                );
            }
        }

        // Clean, exact parse with derived commands → queue them for review. Diagnostics are atomic
        // failures above; a successful batch cannot contain silently skipped graph behavior.
        if super::frontend_tool_bridge::scoped_tool_execution_cancelled() {
            return cancelled_result();
        }
        let queued_count = result.commands.len();
        if let Some(store) = &side_effect_commands
            && let Ok(mut commands) = store.lock()
        {
            if !commands.extend(result.commands) {
                return ToolResultObject::text(
                    json!({
                        "status": "error",
                        "code": "COMMAND_DELIVERY_CONFLICT",
                        "retryable": false,
                        "queued_count": 0,
                        "flowscript_workspace_summary": flowscript_summary(flowscript),
                        "message": "This response already carries an exact retained workflow review. Legacy FlowScript commands were refused rather than mixing them under that review token; finish the existing Apply/Dismiss review."
                    })
                    .to_string(),
                );
            }
        }
        if let Some(store) = &queued_flowscript
            && let Ok(mut workspace) = store.lock()
        {
            *workspace = Some(flowscript.to_string());
        }
        let payload = json!({
            "status": "queued",
            "retryable": false,
            "next_action": "stop",
            "queued_count": queued_count,
            "explanation": format!("Reconciled {queued_count} change(s) from edited FlowScript."),
            "diagnostics": result.diagnostics,
            "structured_diagnostics": structured_diagnostics,
            "flowscript_workspace_summary": flowscript_summary(flowscript),
            "message": format!("Queued {queued_count} board change(s) for user review. Stop now; do not search or submit this FlowScript again."),
        });
        ToolResultObject::text(serde_json::to_string_pretty(&payload).unwrap_or_default())
    });

    (tool, handler)
}

/// Get unconfigured nodes - find nodes with empty/unconnected required inputs
fn create_get_unconfigured_nodes_tool(context: Arc<GraphContext>) -> (Tool, ToolHandler) {
    let tool = tool_from_rig_definition(&GetUnconfiguredNodesTool {
        graph_context: context.clone(),
    });

    let handler: ToolHandler = Arc::new(move |_name, _args| {
        ToolResultObject::text(build_unconfigured_nodes_output(&context))
    });

    (tool, handler)
}

/// List board nodes - get a compact overview of all nodes in the workflow
fn create_list_board_nodes_tool(context: Arc<GraphContext>) -> (Tool, ToolHandler) {
    let tool = tool_from_rig_definition(&ListBoardNodesTool {
        graph_context: context.clone(),
    });

    let handler: ToolHandler = Arc::new(move |_name, _args| {
        ToolResultObject::text(build_list_board_nodes_output(&context))
    });

    (tool, handler)
}

// =============================================================================
// FRONTEND (A2UI) TOOLS
// =============================================================================

/// A UI tree successfully emitted via `emit_ui`, captured for transports that cannot parse tool
/// results (the external-agent MCP bridge): the run's response drains the last one into
/// `components`/`canvas_settings`/`root_component_id`.
#[derive(Clone)]
pub struct EmittedSurface {
    pub root_component_id: String,
    pub canvas_settings: Value,
    pub components: Value,
}

/// Create all Copilot SDK tools for frontend/A2UI context.
///
/// `emit_ui` validates internally (an invalid tree is never rendered), so there is no separate
/// validate tool; `get_component_schema` remains as a fallback for components missing from the
/// documentation embedded in the system prompt.
pub fn create_frontend_tools(
    emitted_surfaces: Option<Arc<Mutex<Vec<EmittedSurface>>>>,
) -> Vec<(Tool, ToolHandler)> {
    vec![
        create_get_component_schema_tool(),
        create_emit_ui_tool(emitted_surfaces),
    ]
}

fn emit_ui_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "rootComponentId": {
                "type": "string",
                "description": "ID of the root component"
            },
            "canvasSettings": {
                "type": "object",
                "description": "Canvas settings (backgroundColor, padding, customCss)"
            },
            "components": {
                "type": "array",
                "description": "Array of SurfaceComponent objects",
                "items": { "type": "object" }
            }
        },
        "required": ["rootComponentId", "components"]
    })
}

/// Emit UI tool - output A2UI JSON components
fn create_emit_ui_tool(
    emitted_surfaces: Option<Arc<Mutex<Vec<EmittedSurface>>>>,
) -> (Tool, ToolHandler) {
    let tool = Tool::new("emit_ui")
        .description(
            r#"Output A2UI components to render in the interface. This is NOT file editing - it generates JSON that renders directly in the app.

emit_ui validates before rendering: an invalid component tree renders nothing and the errors are
returned — fix them and call emit_ui again.

OUTPUT FORMAT:
{
  "rootComponentId": "root",
  "canvasSettings": { "backgroundColor": "bg-background", "padding": "1rem" },
  "components": [...]
}

COMPONENT FORMAT:
{
  "id": "unique-kebab-case-id",
  "style": { "className": "tailwind classes" },
  "component": { "type": "componentType", ...props }
}

BOUNDVALUE FORMAT (ALL props use this):
- String: {"literalString": "text"}
- Number: {"literalNumber": 42}
- Boolean: {"literalBool": true}
- Options: {"literalOptions": [{"value": "v", "label": "L"}]}
- Data binding: {"path": "$.data.field", "defaultValue": "fallback"}

CHILDREN FORMAT:
"children": {"explicitList": ["child-id-1", "child-id-2"]}

AVAILABLE COMPONENTS:
Layout: column, row, grid, stack, scrollArea, box, center, spacer
Display: text, image, icon, badge, avatar, progress, spinner, divider, markdown, diffView
Interactive: button, textField, select, slider, checkbox, switch, link
Container: card, modal, tabs, accordion, drawer, tooltip

THEME COLORS (use these, not hardcoded):
bg-background, bg-muted, bg-card, bg-primary, bg-secondary
text-foreground, text-muted-foreground, text-primary-foreground
border-border

CUSTOM CSS (for advanced effects):
Use canvasSettings.customCss for animations/effects not achievable with Tailwind:
{"canvasSettings": {"backgroundColor": "bg-background", "customCss": ".animated { animation: fade 1s; } @keyframes fade { from{opacity:0} to{opacity:1} }"}}

EXAMPLE - Simple card:
{
  "rootComponentId": "card-1",
  "canvasSettings": {"backgroundColor": "bg-background"},
  "components": [
    {
      "id": "card-1",
      "style": {"className": "p-4"},
      "component": {
        "type": "card",
        "children": {"explicitList": ["title", "content"]}
      }
    },
    {
      "id": "title",
      "component": {
        "type": "text",
        "content": {"literalString": "Hello"},
        "variant": {"literalString": "h2"}
      }
    },
    {
      "id": "content",
      "component": {
        "type": "text",
        "content": {"literalString": "World"}
      }
    }
  ]
}"#,
        )
        .schema(emit_ui_schema());

    let handler: ToolHandler = Arc::new(move |_name, args| {
        let root_id = args
            .get("rootComponentId")
            .and_then(|v| v.as_str())
            .unwrap_or("root");
        let canvas = args.get("canvasSettings").cloned().unwrap_or(json!({}));
        let components = args.get("components").cloned().unwrap_or(json!([]));

        // Validate components and collect errors
        let (validated_components, validation_errors) =
            validate_ui_components(root_id, &canvas, &components);

        if !validation_errors.is_empty() {
            let error_list = validation_errors.join("\n- ");
            // No tree echo: the errors name the offending component ids, and the model already
            // has the tree it submitted. Echoing 100+ components per retry drowns the loop.
            let result = json!({
                "status": "validation_errors",
                "errors": validation_errors,
                "rootComponentId": root_id,
                "message": format!(
                    "Nothing was rendered — {} validation error(s). Fix these and call emit_ui again with the full corrected tree:\n- {}",
                    validation_errors.len(),
                    error_list
                )
            });
            return ToolResultObject::text(serde_json::to_string(&result).unwrap_or_default());
        }

        let component_count = validated_components
            .as_array()
            .map(|components| components.len())
            .unwrap_or_default();

        // The rendered tree travels through the emitted-surfaces store (the chat loop drains it
        // into a <components> frame); the model only needs the outcome. Without a store (no
        // consumer to drain it), fall back to echoing the tree so it is not lost.
        let result = match &emitted_surfaces {
            Some(store) => {
                if let Ok(mut surfaces) = store.lock() {
                    surfaces.push(EmittedSurface {
                        root_component_id: root_id.to_string(),
                        canvas_settings: canvas.clone(),
                        components: validated_components,
                    });
                }
                json!({
                    "status": "rendered",
                    "rootComponentId": root_id,
                    "component_count": component_count,
                    "message": format!("Rendered {component_count} UI component(s) successfully.")
                })
            }
            None => json!({
                "status": "rendered",
                "rootComponentId": root_id,
                "canvasSettings": canvas,
                "components": validated_components,
                "message": "UI components have been rendered successfully"
            }),
        };

        ToolResultObject::text(serde_json::to_string(&result).unwrap_or_default())
    });

    (tool, handler)
}

/// Component schema lookup tool
fn create_get_component_schema_tool() -> (Tool, ToolHandler) {
    let tool = Tool::new("get_component_schema")
        .description(
            r#"FALLBACK detail lookup for a few A2UI component types. The component documentation embedded in your system prompt is the authoritative reference — do NOT call this for components already documented there.

Returns: Full property list with types, required fields, BoundValue format, and a working example.

DETAILED PAGES EXIST ONLY FOR:
column, row, grid, text, button, feedback, appLink, card, userProfile, textField, select, image,
icon, diffView, calendar, gantt, checkbox, switch, tabs, modal
Style categories: spacing, colors, effects, layout, responsive, typography
All other types return a pointer back to the embedded documentation."#,
        )
        .schema(json!({
            "type": "object",
            "properties": {
                "component_types": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Array of component type names to look up (e.g., [\"card\", \"text\", \"button\"])"
                }
            },
            "required": ["component_types"]
        }));

    let handler: ToolHandler = Arc::new(move |_name, args| {
        let types = args
            .get("component_types")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        if types.is_empty() {
            return ToolResultObject::text(
                "Please provide at least one component type to look up.",
            );
        }

        let mut docs = Vec::new();
        for comp_type in &types {
            docs.push(format!(
                "## {}\n{}",
                comp_type,
                get_component_schema_doc(comp_type)
            ));
        }

        ToolResultObject::text(docs.join("\n\n---\n\n"))
    });

    (tool, handler)
}

// =============================================================================
// VALIDATION HELPERS
// =============================================================================

/// Known props per component type (mirrors validateComponents.ts)
fn known_props_for_type(component_type: &str) -> Option<&'static [&'static str]> {
    match component_type {
        "row" => Some(&["gap", "align", "justify", "wrap", "reverse"]),
        "column" => Some(&["gap", "align", "justify", "reverse", "wrap"]),
        "stack" => Some(&["align", "width", "height"]),
        "grid" => Some(&["columns", "rows", "gap", "columnGap", "rowGap", "autoFlow"]),
        "scrollArea" => Some(&["direction"]),
        "aspectRatio" => Some(&["ratio"]),
        "absolute" => Some(&["width", "height"]),
        "box" => Some(&["as"]),
        "center" => Some(&["inline"]),
        "spacer" => Some(&["size", "flex"]),
        "overlay" => Some(&["baseComponentId", "overlays"]),
        // Mirrors WidgetInstanceComponent in packages/ui/components/a2ui/types.ts plus the
        // inline-definition form the prompts document (A2UIWidgetInstance.tsx).
        "widgetInstance" => Some(&[
            "instanceId",
            "widgetId",
            "appId",
            "inlineWidgetDef",
            "exposedPropValues",
            "actionBindings",
            "styleOverride",
        ]),
        "text" => Some(&[
            "content", "variant", "size", "weight", "color", "align", "truncate", "maxLines",
        ]),
        "image" => Some(&["src", "alt", "fit", "fallback", "loading", "aspectRatio"]),
        "icon" => Some(&["name", "size", "color", "strokeWidth"]),
        "video" => Some(&[
            "src", "poster", "autoplay", "loop", "muted", "controls", "width", "height",
        ]),
        "lottie" => Some(&["src", "autoplay", "loop", "speed", "width", "height"]),
        "markdown" => Some(&["content", "allowHtml"]),
        "diffView" => Some(&[
            "original",
            "modified",
            "mode",
            "kind",
            "language",
            "markdownMode",
            "showLineNumbers",
            "wordWrap",
            "wordLevel",
            "collapseUnchanged",
            "contextLines",
            "showStats",
            "originalLabel",
            "modifiedLabel",
            "ignoreWhitespace",
            "ignoreCase",
            "trimTrailingWhitespace",
            "swapSides",
        ]),
        "divider" => Some(&["orientation", "thickness", "color"]),
        "badge" => Some(&["content", "variant", "color"]),
        "avatar" => Some(&["src", "fallback", "size"]),
        "userProfile" => Some(&[
            "value",
            "variant",
            "avatarSize",
            "showHover",
            "showEmail",
            "showDescription",
            "showUserId",
            "showProfileLink",
            "fallbackLabel",
            "muted",
        ]),
        "progress" => Some(&["value", "max", "showLabel", "variant", "color"]),
        "spinner" => Some(&["size", "color"]),
        "skeleton" => Some(&["width", "height", "rounded"]),
        "iframe" => Some(&[
            "src",
            "srcdoc",
            "width",
            "height",
            "sandbox",
            "allow",
            "title",
            "referrerPolicy",
            "border",
            "loading",
        ]),
        "table" => Some(&[
            "columns",
            "data",
            "caption",
            "striped",
            "bordered",
            "hoverable",
            "compact",
            "stickyHeader",
            "sortable",
            "searchable",
            "paginated",
            "pageSize",
            "selectable",
            "onRowClick",
        ]),
        "tableRow" => Some(&["cells", "selected", "disabled"]),
        "tableCell" => Some(&["content", "isHeader", "colSpan", "rowSpan", "align"]),
        "plotlyChart" => Some(&[
            "chartType",
            "title",
            "series",
            "xAxis",
            "yAxis",
            "data",
            "layout",
            "config",
            "width",
            "height",
            "responsive",
            "showLegend",
            "legendPosition",
        ]),
        "nivoChart" => Some(&[
            "chartType",
            "title",
            "data",
            "height",
            "colors",
            "animate",
            "showLegend",
            "legendPosition",
            "indexBy",
            "keys",
            "margin",
            "axisBottom",
            "axisLeft",
            "axisTop",
            "axisRight",
            "config",
            "barStyle",
            "lineStyle",
            "pieStyle",
            "radarStyle",
            "heatmapStyle",
            "scatterStyle",
            "funnelStyle",
            "treemapStyle",
            "sankeyStyle",
            "calendarStyle",
            "chordStyle",
        ]),
        "filePreview" => Some(&[
            "src",
            "url",
            "filename",
            "mimeType",
            "fileType",
            "showControls",
            "fit",
            "fallbackText",
            "height",
            "showDownload",
            "loading",
            "variant",
            "autoPlay",
        ]),
        "boundingBoxOverlay" => Some(&[
            "src",
            "alt",
            "boxes",
            "showLabels",
            "showConfidence",
            "strokeWidth",
            "fontSize",
            "fit",
            "normalized",
            "interactive",
        ]),
        "button" => Some(&[
            "label",
            "variant",
            "size",
            "disabled",
            "loading",
            "icon",
            "iconPosition",
            "tooltip",
        ]),
        "feedback" => Some(&[
            "mode",
            "size",
            "title",
            "description",
            "positiveLabel",
            "negativeLabel",
            "positiveRating",
            "negativeRating",
            "showComment",
            "commentMode",
            "commentLabel",
            "commentPlaceholder",
            "commentTitle",
            "commentDescription",
            "commentSubmitLabel",
            "commentCancelLabel",
            "feedbackId",
            "includeState",
            "pageContextMode",
            "pageContextQueryParamAllowlist",
            "pageContextQueryParamDenylist",
            "includePageHash",
            "successMessage",
            "disabled",
        ]),
        "appLink" => Some(&[
            "target",
            "label",
            "variant",
            "size",
            "icon",
            "iconPosition",
            "appId",
            "eventId",
            "disabled",
        ]),
        "textField" => Some(&[
            "value",
            "placeholder",
            "label",
            "helperText",
            "error",
            "disabled",
            "inputType",
            "multiline",
            "rows",
            "maxLength",
            "required",
        ]),
        "select" => Some(&[
            "value",
            "options",
            "placeholder",
            "label",
            "disabled",
            "multiple",
            "searchable",
        ]),
        "slider" => Some(&[
            "value",
            "min",
            "max",
            "step",
            "disabled",
            "showValue",
            "label",
        ]),
        "checkbox" => Some(&["checked", "label", "disabled", "indeterminate"]),
        "switch" => Some(&["checked", "label", "disabled"]),
        "radioGroup" => Some(&["value", "options", "disabled", "orientation", "label"]),
        "dateTimeInput" => Some(&["value", "mode", "min", "max", "disabled", "label"]),
        "fileInput" => Some(&[
            "value",
            "label",
            "helperText",
            "accept",
            "multiple",
            "maxSize",
            "maxFiles",
            "disabled",
            "error",
        ]),
        "imageInput" => Some(&[
            "value",
            "label",
            "helperText",
            "accept",
            "multiple",
            "maxSize",
            "maxFiles",
            "disabled",
            "error",
            "aspectRatio",
            "showPreview",
        ]),
        "voiceInput" => Some(&[
            "value",
            "label",
            "helperText",
            "maxDuration",
            "autoStop",
            "silenceThreshold",
            "silenceDuration",
            "disabled",
            "error",
            "visualizer",
            "variant",
            "size",
            "mode",
            "invoke",
            "color",
            "recordingColor",
            "resultMode",
            "src",
            "url",
        ]),
        "imageLabeler" => Some(&[
            "src",
            "alt",
            "boxes",
            "labels",
            "disabled",
            "showLabels",
            "minBoxSize",
        ]),
        "imageHotspot" => Some(&[
            "src",
            "alt",
            "hotspots",
            "showMarkers",
            "markerStyle",
            "fit",
            "normalized",
            "showTooltips",
        ]),
        "geoMap" => Some(&[
            "viewport",
            "markers",
            "routes",
            "showControls",
            "showZoom",
            "showCompass",
            "showLocate",
            "showFullscreen",
            "interactive",
            "controlPosition",
            "clusterMarkers",
            "clusterRadius",
            "clusterMaxZoom",
        ]),
        "link" => Some(&[
            "href",
            "label",
            "route",
            "queryParams",
            "external",
            "target",
            "variant",
            "underline",
            "disabled",
        ]),
        "card" => Some(&[
            "title",
            "description",
            "footer",
            "hoverable",
            "clickable",
            "variant",
            "padding",
            "headerImage",
            "headerIcon",
        ]),
        "modal" => Some(&[
            "open",
            "title",
            "description",
            "closeOnOverlay",
            "closeOnEscape",
            "showCloseButton",
            "size",
            "centered",
        ]),
        "tabs" => Some(&[
            "value",
            "tabs",
            "orientation",
            "variant",
            "listStyle",
            "triggerStyle",
            "contentStyle",
        ]),
        "accordion" => Some(&["items", "multiple", "defaultExpanded", "collapsible"]),
        "drawer" => Some(&["open", "side", "title", "size", "overlay", "closable"]),
        "tooltip" => Some(&["content", "side", "delayMs", "maxWidth"]),
        "popover" => Some(&[
            "open",
            "contentComponentId",
            "side",
            "trigger",
            "closeOnClickOutside",
        ]),
        "canvas2d" => Some(&["width", "height", "backgroundColor", "pixelPerfect"]),
        "sprite" => Some(&[
            "src", "x", "y", "width", "height", "rotation", "scale", "opacity", "flipX", "flipY",
            "zIndex",
        ]),
        "shape" => Some(&[
            "shapeType",
            "x",
            "y",
            "width",
            "height",
            "radius",
            "points",
            "fill",
            "stroke",
            "strokeWidth",
        ]),
        "scene3d" => Some(&[
            "width",
            "height",
            "cameraType",
            "cameraPosition",
            "backgroundColor",
            "controlMode",
            "fixedView",
            "autoRotateSpeed",
            "enableControls",
            "enableZoom",
            "enablePan",
            "fov",
            "near",
            "far",
            "target",
            "ambientLight",
            "directionalLight",
            "showGrid",
            "showAxes",
        ]),
        "model3d" => Some(&[
            "src",
            "position",
            "rotation",
            "scale",
            "castShadow",
            "receiveShadow",
            "animation",
            "autoRotate",
            "rotateSpeed",
            "viewerHeight",
            "backgroundColor",
            "cameraDistance",
            "fov",
            "cameraAngle",
            "cameraPosition",
            "cameraTarget",
            "enableControls",
            "enableZoom",
            "enablePan",
            "autoRotateCamera",
            "cameraRotateSpeed",
            "ambientLight",
            "directionalLight",
            "fillLight",
            "rimLight",
            "lightColor",
            "lightingPreset",
            "showGround",
            "groundColor",
            "enableReflections",
            "environment",
            "environmentSource",
            "useHdrBackground",
            "polyhavenHdri",
            "polyhavenResolution",
            "hdriUrl",
            "groundSize",
            "groundOffsetY",
            "groundFollowCamera",
        ]),
        "dialogue" => Some(&[
            "text",
            "speakerName",
            "speakerPortraitId",
            "typewriter",
            "typewriterSpeed",
        ]),
        "characterPortrait" => Some(&["image", "expression", "position", "size", "dimmed"]),
        "choiceMenu" => Some(&["choices", "title", "layout"]),
        "inventoryGrid" => Some(&["items", "columns", "rows", "cellSize"]),
        "healthBar" => Some(&[
            "value",
            "maxValue",
            "label",
            "showValue",
            "fillColor",
            "backgroundColor",
            "variant",
        ]),
        "miniMap" => Some(&[
            "mapImage",
            "width",
            "height",
            "markers",
            "playerX",
            "playerY",
            "playerRotation",
        ]),
        "calendar" => Some(&[
            "events",
            "view",
            "date",
            "title",
            "density",
            "editable",
            "selectable",
            "firstDayOfWeek",
            "minTime",
            "maxTime",
            "slotDuration",
            "showWeekends",
            "showNowIndicator",
            "showAllDay",
            "showViewSwitcher",
            "locale",
            "height",
            "responsive",
            "compactBreakpoint",
        ]),
        "gantt" => Some(&[
            "tasks",
            "view",
            "title",
            "density",
            "editable",
            "draggable",
            "resizable",
            "showDependencies",
            "showProgress",
            "showToday",
            "showViewSwitcher",
            "showTaskList",
            "taskListWidth",
            "shadeWeekends",
            "rowHeight",
            "columns",
            "height",
            "responsive",
            "compactBreakpoint",
        ]),
        _ => None,
    }
}

/// Required props per component type
fn required_props_for_type(component_type: &str) -> &'static [&'static str] {
    match component_type {
        "text" => &["content"],
        "image" => &["src"],
        "icon" => &["name"],
        "video" => &["src"],
        "lottie" => &["src"],
        "markdown" => &["content"],
        "diffView" => &["original", "modified"],
        "badge" => &["content"],
        "userProfile" => &["value"],
        "progress" => &["value"],
        "button" => &["label"],
        "textField" => &["value"],
        "select" => &["value", "options"],
        "slider" => &["value"],
        "checkbox" => &["checked"],
        "switch" => &["checked"],
        "radioGroup" => &["value", "options"],
        "dateTimeInput" => &["value"],
        "fileInput" => &["value"],
        "imageInput" => &["value"],
        "link" => &["href"],
        "modal" => &["open"],
        "tabs" => &["value"],
        "canvas2d" => &["width", "height"],
        "sprite" => &["src", "x", "y"],
        "shape" => &["shapeType", "x", "y"],
        "scene3d" => &["width", "height"],
        "model3d" => &["src"],
        "aspectRatio" => &["ratio"],
        "boundingBoxOverlay" => &["src"],
        "calendar" => &["events"],
        "gantt" => &["tasks"],
        _ => &[],
    }
}

const BASE_PROPS: &[&str] = &["type", "id", "style", "children", "actions", "hidden"];
const MAX_UI_COMPONENTS: usize = 120;
const MAX_UI_COMPONENT_ID_CHARS: usize = 120;
const MAX_UI_CUSTOM_CSS_CHARS: usize = 12_000;
const MAX_UI_STYLE_STRING_CHARS: usize = 1_000;
const MAX_UI_ACTIONS: usize = 20;

/// Validate an array of components and return (validated_components, errors)
fn validate_ui_components(
    root_id: &str,
    canvas: &Value,
    components: &Value,
) -> (Value, Vec<String>) {
    let mut errors = Vec::new();
    validate_canvas_settings(canvas, &mut errors);

    let arr = match components.as_array() {
        Some(a) => a,
        None => {
            errors.push("'components' must be an array".to_string());
            return (json!([]), errors);
        }
    };

    if arr.len() > MAX_UI_COMPONENTS {
        errors.push(format!(
            "'components' is limited to {MAX_UI_COMPONENTS} components per response"
        ));
    }

    let mut all_ids = HashSet::new();
    let mut duplicate_ids = HashSet::new();
    for comp in arr {
        if let Some(id) = comp.get("id").and_then(|v| v.as_str())
            && !all_ids.insert(id.to_string())
        {
            duplicate_ids.insert(id.to_string());
        }
    }
    for id in &duplicate_ids {
        errors.push(format!("Duplicate component id '{}'", id));
    }
    if !root_id.is_empty() && !all_ids.contains(root_id) {
        errors.push(format!(
            "rootComponentId '{}' does not exist in the components array",
            root_id
        ));
    }

    let mut validated = Vec::new();
    let mut child_graph: HashMap<String, Vec<String>> = HashMap::new();

    for comp in arr {
        let id = match comp.get("id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => {
                errors.push(
                    "Component missing 'id' field - every component needs a unique id".to_string(),
                );
                continue;
            }
        };
        if id.trim().is_empty() {
            errors.push("Component ids cannot be empty".to_string());
            continue;
        }
        if id.chars().count() > MAX_UI_COMPONENT_ID_CHARS {
            errors.push(format!(
                "{}: component id is too long; maximum is {MAX_UI_COMPONENT_ID_CHARS} characters",
                id
            ));
            continue;
        }

        let component = match comp.get("component") {
            Some(c) if c.is_object() => c,
            _ => {
                errors.push(format!("{}: missing 'component' object", id));
                continue;
            }
        };

        let comp_type = match component.get("type").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => {
                errors.push(format!("{}: missing 'component.type' field", id));
                continue;
            }
        };

        let known = known_props_for_type(comp_type);
        if known.is_none() {
            errors.push(format!(
                "{}: unknown component type '{}'. Use one of the component types listed in the component documentation in your system prompt.",
                id, comp_type
            ));
            continue;
        }
        let known_set = known.unwrap();

        if comp_type == "markdown"
            && component
                .get("allowHtml")
                .and_then(|value| value.get("literalBool"))
                .and_then(|value| value.as_bool())
                == Some(true)
        {
            errors.push(format!(
                "{}: markdown.allowHtml must be false for generated UI",
                id
            ));
        }

        if comp_type == "iframe"
            && let Some(sandbox) = component
                .get("sandbox")
                .and_then(|value| value.get("literalString"))
                .and_then(|value| value.as_str())
        {
            for token in ["allow-same-origin", "allow-popups-to-escape-sandbox"] {
                if sandbox.split_whitespace().any(|part| part == token) {
                    errors.push(format!(
                        "{}: iframe sandbox token '{}' is not allowed in generated UI",
                        id, token
                    ));
                }
            }
        }

        if let Some(obj) = component.as_object() {
            for key in obj.keys() {
                let k = key.as_str();
                if !BASE_PROPS.contains(&k) && !known_set.contains(&k) {
                    errors.push(format!(
                        "{}: unknown prop '{}' on '{}'. Valid props: {}. Check the component documentation in your system prompt.",
                        id,
                        k,
                        comp_type,
                        known.map(|props| props.join(", ")).unwrap_or_default()
                    ));
                }
            }
        }

        for required in required_props_for_type(comp_type) {
            if component.get(*required).is_none() {
                errors.push(format!(
                    "{}: missing required prop '{}' on '{}'. This prop is mandatory.",
                    id, required, comp_type
                ));
            }
        }

        // Props that are plain values in a2ui/types.ts (not BoundValue-wrapped).
        let plain_props: &[&str] = match comp_type {
            "overlay" => &["baseComponentId"],
            "popover" => &["contentComponentId"],
            "widgetInstance" => &["instanceId", "widgetId", "appId"],
            "link" => &["external", "target", "variant", "underline"],
            _ => &[],
        };

        if let Some(obj) = component.as_object() {
            for (key, value) in obj {
                let k = key.as_str();
                if BASE_PROPS.contains(&k) || plain_props.contains(&k) {
                    continue;
                }
                if matches!(
                    k,
                    "tabs"
                        | "items"
                        | "overlays"
                        | "columns"
                        | "data"
                        | "boxes"
                        | "hotspots"
                        | "markers"
                        | "choices"
                ) {
                    continue;
                }
                if (value.is_string() || value.is_number() || value.is_boolean())
                    && known_set.contains(&k)
                    && k != "type"
                {
                    errors.push(format!(
                            "{}: prop '{}' uses a bare value. Wrap it as BoundValue: string→{{\"literalString\": \"{}\"}}, number→{{\"literalNumber\": {}}}, bool→{{\"literalBool\": {}}}",
                            id, k,
                            value.as_str().unwrap_or("..."),
                            value.as_f64().map(|n| n.to_string()).unwrap_or_else(|| "...".to_string()),
                            value.as_bool().map(|b| b.to_string()).unwrap_or_else(|| "...".to_string()),
                        ));
                }
            }
        }

        if let Some(style) = comp.get("style") {
            validate_style_value(id, "style", style, &mut errors);
        }
        if let Some(style) = component.get("style") {
            validate_style_value(id, "component.style", style, &mut errors);
        }
        if let Some(actions) = component.get("actions") {
            validate_actions_value(id, actions, &mut errors);
        }

        let mut component_refs = Vec::new();
        if let Some(children) = component.get("children") {
            component_refs.extend(collect_child_refs(id, children, &all_ids, &mut errors));
        }

        if let Some(content_component_id) = component
            .get("contentComponentId")
            .and_then(bound_or_plain_string)
        {
            push_component_ref(
                id,
                content_component_id,
                "contentComponentId",
                &all_ids,
                &mut errors,
                &mut component_refs,
            );
        }

        if let Some(base_component_id) = component
            .get("baseComponentId")
            .and_then(bound_or_plain_string)
        {
            push_component_ref(
                id,
                base_component_id,
                "baseComponentId",
                &all_ids,
                &mut errors,
                &mut component_refs,
            );
        }

        if let Some(overlays) = component.get("overlays").and_then(|value| value.as_array()) {
            for overlay in overlays {
                if let Some(overlay_id) = overlay
                    .get("componentId")
                    .or_else(|| overlay.get("id"))
                    .and_then(bound_or_plain_string)
                {
                    push_component_ref(
                        id,
                        overlay_id,
                        "overlays[].componentId",
                        &all_ids,
                        &mut errors,
                        &mut component_refs,
                    );
                }
            }
        }

        for (array_prop, ref_prop) in [
            ("tabs", "contentComponentId"),
            ("items", "contentComponentId"),
        ] {
            if let Some(items) = component.get(array_prop).and_then(|value| value.as_array()) {
                for item in items {
                    if let Some(content_component_id) =
                        item.get(ref_prop).and_then(bound_or_plain_string)
                    {
                        push_component_ref(
                            id,
                            content_component_id,
                            &format!("{array_prop}[].{ref_prop}"),
                            &all_ids,
                            &mut errors,
                            &mut component_refs,
                        );
                    }
                }
            }
        }

        if !component_refs.is_empty() {
            child_graph.insert(id.to_string(), component_refs);
        }

        validated.push(comp.clone());
    }

    if let Some(cycle) = find_child_cycle(&child_graph) {
        errors.push(format!(
            "Component references contain a cycle: {}",
            cycle.join(" -> ")
        ));
    }

    (json!(validated), errors)
}

fn bound_or_plain_string(value: &Value) -> Option<&str> {
    value
        .get("literalString")
        .and_then(Value::as_str)
        .or_else(|| value.as_str())
}

fn push_component_ref(
    parent_id: &str,
    target_id: &str,
    field: &str,
    all_ids: &HashSet<String>,
    errors: &mut Vec<String>,
    refs: &mut Vec<String>,
) {
    if target_id == parent_id {
        errors.push(format!("{}: {} cannot reference itself", parent_id, field));
    }
    if !all_ids.contains(target_id) {
        errors.push(format!(
            "{}: {} references '{}' which doesn't exist",
            parent_id, field, target_id
        ));
    }
    refs.push(target_id.to_string());
}

fn is_known_style_prop(key: &str) -> bool {
    matches!(
        key,
        "className"
            | "background"
            | "border"
            | "shadow"
            | "position"
            | "transform"
            | "overflow"
            | "responsiveOverrides"
            | "margin"
            | "padding"
            | "gap"
            | "width"
            | "height"
            | "minWidth"
            | "minHeight"
            | "maxWidth"
            | "maxHeight"
            | "flex"
            | "flexGrow"
            | "flexShrink"
            | "flexBasis"
            | "alignSelf"
            | "gridColumn"
            | "gridRow"
            | "gridArea"
            | "justifySelf"
            | "color"
            | "fontSize"
            | "fontWeight"
            | "fontFamily"
            | "lineHeight"
            | "letterSpacing"
            | "textAlign"
            | "textDecoration"
            | "textTransform"
            | "whiteSpace"
            | "wordBreak"
            | "opacity"
            | "visibility"
            | "cursor"
            | "userSelect"
            | "pointerEvents"
            | "zIndex"
            | "transition"
            | "animation"
            | "display"
            | "outline"
            | "outlineOffset"
            | "filter"
            | "backdropFilter"
            | "aspectRatio"
    )
}

fn validate_style_value(component_id: &str, path: &str, style: &Value, errors: &mut Vec<String>) {
    let Some(style_obj) = style.as_object() else {
        errors.push(format!("{}: {} must be an object", component_id, path));
        return;
    };

    for (key, value) in style_obj {
        if !is_known_style_prop(key) {
            errors.push(format!(
                "{}: unknown style prop '{}.{}'",
                component_id, path, key
            ));
        }
        validate_style_strings(component_id, &format!("{path}.{key}"), value, errors);
    }
}

fn validate_style_strings(component_id: &str, path: &str, value: &Value, errors: &mut Vec<String>) {
    match value {
        Value::String(text) => {
            if text.len() > MAX_UI_STYLE_STRING_CHARS {
                errors.push(format!(
                    "{}: {} is too long; maximum is {MAX_UI_STYLE_STRING_CHARS} bytes",
                    component_id, path
                ));
            }

            let lowered = text.to_ascii_lowercase();
            let compact: String = lowered.chars().filter(|ch| !ch.is_whitespace()).collect();
            if compact.contains("javascript:")
                || compact.contains("vbscript:")
                || compact.contains("data:text/html")
                || compact.contains("-moz-binding")
            {
                errors.push(format!(
                    "{}: {} contains an unsafe CSS value",
                    component_id, path
                ));
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                validate_style_strings(component_id, &format!("{path}[{index}]"), item, errors);
            }
        }
        Value::Object(obj) => {
            for (key, item) in obj {
                validate_style_strings(component_id, &format!("{path}.{key}"), item, errors);
            }
        }
        _ => {}
    }
}

fn validate_actions_value(component_id: &str, actions: &Value, errors: &mut Vec<String>) {
    let Some(actions) = actions.as_array() else {
        errors.push(format!("{}: actions must be an array", component_id));
        return;
    };

    if actions.len() > MAX_UI_ACTIONS {
        errors.push(format!(
            "{}: actions is limited to {MAX_UI_ACTIONS} entries",
            component_id
        ));
    }

    for (index, action) in actions.iter().enumerate() {
        let Some(action_obj) = action.as_object() else {
            errors.push(format!(
                "{}: actions[{index}] must be an object",
                component_id
            ));
            continue;
        };

        for key in action_obj.keys() {
            if !matches!(key.as_str(), "name" | "context") {
                errors.push(format!(
                    "{}: unknown action prop 'actions[{index}].{}'",
                    component_id, key
                ));
            }
        }

        match action_obj.get("name").and_then(Value::as_str) {
            Some(name) if !name.trim().is_empty() => {}
            _ => errors.push(format!(
                "{}: actions[{index}].name must be a non-empty string",
                component_id
            )),
        }

        match action_obj.get("context") {
            Some(Value::Object(_)) => {}
            Some(_) => errors.push(format!(
                "{}: actions[{index}].context must be an object",
                component_id
            )),
            None => errors.push(format!(
                "{}: actions[{index}].context is required",
                component_id
            )),
        }
    }
}

fn validate_canvas_settings(canvas: &Value, errors: &mut Vec<String>) {
    if !canvas.is_object() {
        if !canvas.is_null() {
            errors.push("canvasSettings must be an object".to_string());
        }
        return;
    }
    if let Some(custom_css) = canvas.get("customCss").and_then(|value| value.as_str())
        && custom_css.len() > MAX_UI_CUSTOM_CSS_CHARS
    {
        errors.push(format!(
            "canvasSettings.customCss is too large; maximum is {MAX_UI_CUSTOM_CSS_CHARS} bytes"
        ));
    }
    if let Some(background_image) = canvas
        .get("backgroundImage")
        .and_then(|value| value.as_str())
    {
        let allowed = background_image.starts_with("http://")
            || background_image.starts_with("https://")
            || background_image.starts_with("data:image/png;base64,")
            || background_image.starts_with("data:image/jpeg;base64,")
            || background_image.starts_with("data:image/webp;base64,")
            || background_image.starts_with("data:image/gif;base64,");
        if !allowed {
            errors.push(
                "canvasSettings.backgroundImage must be http(s) or a safe data:image URL"
                    .to_string(),
            );
        }
    }
}

fn collect_child_refs(
    parent_id: &str,
    children: &Value,
    all_ids: &HashSet<String>,
    errors: &mut Vec<String>,
) -> Vec<String> {
    let Some(children_obj) = children.as_object() else {
        errors.push(format!("{}: children must be an object", parent_id));
        return Vec::new();
    };

    if let Some(explicit_list) = children_obj.get("explicitList") {
        let Some(explicit_list) = explicit_list.as_array() else {
            errors.push(format!(
                "{}: children.explicitList must be an array of component ids",
                parent_id
            ));
            return Vec::new();
        };

        let mut refs = Vec::new();
        for child_ref in explicit_list {
            let Some(child_id) = child_ref.as_str() else {
                errors.push(format!(
                    "{}: children.explicitList can only contain strings",
                    parent_id
                ));
                continue;
            };
            if child_id == parent_id {
                errors.push(format!("{}: component cannot be its own child", parent_id));
            }
            if !all_ids.contains(child_id) {
                errors.push(format!(
                    "{}: children references '{}' which doesn't exist in the components array",
                    parent_id, child_id
                ));
            }
            refs.push(child_id.to_string());
        }
        return refs;
    }

    if let Some(template) = children_obj.get("template") {
        let template_component_id = template
            .get("templateComponentId")
            .and_then(|value| value.as_str());
        let data_path = template.get("dataPath").and_then(|value| value.as_str());
        match (template_component_id, data_path) {
            (Some(component_id), Some(_)) if all_ids.contains(component_id) => {
                return vec![component_id.to_string()];
            }
            (Some(component_id), Some(_)) => {
                errors.push(format!(
                    "{}: templateComponentId '{}' does not exist",
                    parent_id, component_id
                ));
            }
            _ => {
                errors.push(format!(
                    "{}: children.template requires templateComponentId and dataPath",
                    parent_id
                ));
            }
        }
        return Vec::new();
    }

    errors.push(format!(
        "{}: children must contain explicitList or template",
        parent_id
    ));
    Vec::new()
}

fn find_child_cycle(graph: &HashMap<String, Vec<String>>) -> Option<Vec<String>> {
    fn visit(
        node: &str,
        graph: &HashMap<String, Vec<String>>,
        visiting: &mut HashSet<String>,
        visited: &mut HashSet<String>,
        stack: &mut Vec<String>,
    ) -> Option<Vec<String>> {
        if visited.contains(node) {
            return None;
        }
        if !visiting.insert(node.to_string()) {
            if let Some(start) = stack.iter().position(|item| item == node) {
                let mut cycle = stack[start..].to_vec();
                cycle.push(node.to_string());
                return Some(cycle);
            }
            return Some(vec![node.to_string(), node.to_string()]);
        }

        stack.push(node.to_string());
        if let Some(children) = graph.get(node) {
            for child in children {
                if let Some(cycle) = visit(child, graph, visiting, visited, stack) {
                    return Some(cycle);
                }
            }
        }
        stack.pop();
        visiting.remove(node);
        visited.insert(node.to_string());
        None
    }

    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    let mut stack = Vec::new();
    for node in graph.keys() {
        if let Some(cycle) = visit(node, graph, &mut visiting, &mut visited, &mut stack) {
            return Some(cycle);
        }
    }
    None
}

/// Get detailed schema documentation for a component type
fn get_component_schema_doc(component_type: &str) -> String {
    use flow_like::a2ui::copilot::get_component_schema;

    let base_doc = get_component_schema(component_type);

    // Add BoundValue reminder and known props list
    if let Some(props) = known_props_for_type(component_type) {
        let required = required_props_for_type(component_type);
        let prop_list: Vec<String> = props
            .iter()
            .map(|p| {
                if required.contains(p) {
                    format!("- {} (REQUIRED)", p)
                } else {
                    format!("- {}", p)
                }
            })
            .collect();

        format!(
            "{}\n\n### Valid Props\n{}\n\n### BoundValue Reminder\nAll props must use BoundValue format:\n- String: {{\"literalString\": \"text\"}}\n- Number: {{\"literalNumber\": 42}}\n- Boolean: {{\"literalBool\": true}}\n- Options: {{\"literalOptions\": [{{\"value\": \"v\", \"label\": \"L\"}}]}}\n- Children: {{\"explicitList\": [\"child-id-1\"]}}",
            base_doc,
            prop_list.join("\n")
        )
    } else {
        base_doc
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::{
        board::{ExecutionMode, ExecutionStage},
        copilot::{
            FlowCapabilityRequirement, FlowIrArg, FlowIrDraftMode, FlowIrLiteral, FlowIrModule,
            FlowIrProgram, FlowIrStep, FlowIrValue, FlowModuleEstimate, FlowModuleKind,
            PinMetadata,
        },
        execution::LogLevel,
        pin::ValueType,
        variable::{Variable, VariableType},
    };
    use flow_like::flow_like_storage::Path;
    use std::time::SystemTime;

    fn empty_board(id: &str) -> Board {
        Board {
            id: id.to_string(),
            name: "Captured".to_string(),
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

    fn typed_catalog() -> Vec<NodeMetadata> {
        vec![
            NodeMetadata {
                name: "events_simple".to_string(),
                friendly_name: "events_simple".to_string(),
                description: String::new(),
                inputs: Vec::new(),
                outputs: vec![pin("exec_out", "Execution")],
                category: None,
                required_inputs: Vec::new(),
                companion_nodes: Vec::new(),
                capability_tags: Vec::new(),
            },
            NodeMetadata {
                name: "string_format".to_string(),
                friendly_name: "string_format".to_string(),
                description: String::new(),
                inputs: vec![pin("format_string", "String")],
                outputs: vec![pin("string", "String")],
                category: None,
                required_inputs: Vec::new(),
                companion_nodes: Vec::new(),
                capability_tags: Vec::new(),
            },
            NodeMetadata {
                name: "log_info".to_string(),
                friendly_name: "log_info".to_string(),
                description: String::new(),
                inputs: vec![pin("exec_in", "Execution"), pin("message", "String")],
                outputs: vec![pin("exec_out", "Execution")],
                category: None,
                required_inputs: vec!["message".to_string()],
                companion_nodes: Vec::new(),
                capability_tags: Vec::new(),
            },
        ]
    }

    struct StaticCatalogProvider {
        catalog: Vec<NodeMetadata>,
    }

    #[async_trait::async_trait]
    impl CatalogProvider for StaticCatalogProvider {
        async fn search(&self, _query: &str) -> Vec<NodeMetadata> {
            self.catalog.clone()
        }

        async fn search_by_pin_type(&self, _pin_type: &str, _is_input: bool) -> Vec<NodeMetadata> {
            self.catalog.clone()
        }

        async fn filter_by_category(&self, _category_prefix: &str) -> Vec<NodeMetadata> {
            self.catalog.clone()
        }

        async fn get_node_metadata(&self, node_type: &str) -> Option<NodeMetadata> {
            self.catalog
                .iter()
                .find(|node| node.name == node_type)
                .cloned()
        }

        async fn get_all_nodes(&self) -> Vec<String> {
            self.catalog.iter().map(|node| node.name.clone()).collect()
        }

        async fn get_all_metadata(&self) -> Vec<NodeMetadata> {
            self.catalog.clone()
        }
    }

    #[test]
    fn board_tool_surface_advertises_source_lifecycle_not_model_authored_json() {
        let board_key = format!("source-tool-surface-{:?}", std::thread::current().id());
        let board = Arc::new(empty_board(&board_key));
        let provider: Arc<dyn CatalogProvider> = Arc::new(StaticCatalogProvider {
            catalog: typed_catalog(),
        });
        let tools = create_board_tools(
            None,
            Some(board),
            None,
            Some("Build an event that formats a message."),
            Some(provider),
            Some(Arc::new(Mutex::new(SideEffectCommandQueue::default()))),
            Some(Arc::new(Mutex::new(None))),
        );
        let names = tools
            .iter()
            .map(|(tool, _)| tool.name.as_str())
            .collect::<HashSet<_>>();

        for expected in [
            "write_flowscript",
            "patch_flowscript",
            "check_flowscript",
            "commit_flowscript",
        ] {
            assert!(names.contains(expected), "missing {expected}: {names:?}");
        }
        for hidden in [
            "plan_flow_ir",
            "begin_flow_ir_draft",
            "update_flow_ir_draft",
            "upsert_flow_ir_module",
            "validate_flow_ir_draft",
            "commit_flow_ir_draft",
            "edit_flowscript",
        ] {
            assert!(!names.contains(hidden), "legacy tool leaked: {hidden}");
        }

        if let Ok(mut stores) = FLOW_IR_DRAFT_STORES.lock() {
            stores.remove(&board_key);
        }
    }

    #[test]
    fn sdk_emit_commands_rejects_executable_json_and_accepts_layout() {
        let queue = Arc::new(Mutex::new(SideEffectCommandQueue::default()));
        let (_, handler) = create_emit_commands_tool(None, None, Some(queue.clone()));

        let rejected = handler(
            "emit_commands",
            &json!({
                "commands": [
                    {
                        "command_type": "AddNode",
                        "node_type": "log_info",
                        "ref_id": "$0",
                        "position": { "x": 0, "y": 0 },
                        "summary": "Add log"
                    },
                    {
                        "command_type": "ConnectPins",
                        "from_node": "start",
                        "from_pin": "exec_out",
                        "to_node": "$0",
                        "to_pin": "exec_in",
                        "summary": "Connect log"
                    }
                ],
                "explanation": "Build executable behavior"
            }),
        );
        let rejected: Value = serde_json::from_str(&rejected.text_result_for_llm)
            .expect("scope rejection is structured JSON");
        assert_eq!(
            rejected["status"], "representation_rejected",
            "{rejected:#}"
        );
        assert_eq!(
            rejected["next_action"],
            "write_patch_check_commit_flowscript"
        );
        assert_eq!(rejected["retry_emit_commands"], false);
        let message = rejected["message"].as_str().expect("redirect message");
        assert!(message.contains("write_flowscript"));
        assert!(message.contains("patch_flowscript"));
        assert!(message.contains("check_flowscript"));
        assert!(message.contains("commit_flowscript"));
        assert!(!message.contains("call emit_commands again"));
        let errors = rejected["errors"].as_array().expect("validation errors");
        assert_eq!(errors.len(), 2);
        assert!(errors.iter().all(|error| {
            error
                .as_str()
                .is_some_and(|error| error.contains("executable-command-requires-flowscript"))
        }));
        let (commands, token) = queue.lock().expect("queue lock").take_delivery();
        assert!(commands.is_empty());
        assert!(token.is_none());

        let accepted = handler(
            "emit_commands",
            &json!({
                "commands": [{
                    "command_type": "MoveNode",
                    "node_id": "node-1",
                    "position": { "x": 120, "y": 80 },
                    "summary": "Align node"
                }],
                "explanation": "Align the workflow"
            }),
        );
        let accepted: Value = serde_json::from_str(&accepted.text_result_for_llm)
            .expect("queued result is structured JSON");
        assert_eq!(accepted["status"], "queued", "{accepted:#}");
        let (commands, token) = queue.lock().expect("queue lock").take_delivery();
        assert_eq!(commands.len(), 1);
        assert!(matches!(
            commands.first(),
            Some(BoardCommand::MoveNode { .. })
        ));
        assert!(token.is_none());
    }

    #[test]
    fn source_commit_queues_only_the_exact_retained_host_batch() {
        let board_key = format!("source-tool-commit-{:?}", std::thread::current().id());
        if let Ok(mut stores) = FLOW_IR_DRAFT_STORES.lock() {
            stores.remove(&board_key);
        }
        let board = Arc::new(empty_board(&board_key));
        let provider: Arc<dyn CatalogProvider> = Arc::new(StaticCatalogProvider {
            catalog: typed_catalog(),
        });
        let queue = Arc::new(Mutex::new(SideEffectCommandQueue::default()));
        let workspace = Arc::new(Mutex::new(None));
        let tools = create_board_tools(
            None,
            Some(board.clone()),
            None,
            Some("Create an event that logs hello."),
            Some(provider),
            Some(queue.clone()),
            Some(workspace.clone()),
        );
        let call = |name: &str, args: Value| {
            let handler = &tools
                .iter()
                .find(|(tool, _)| tool.name == name)
                .unwrap_or_else(|| panic!("missing tool {name}"))
                .1;
            handler(name, &args)
        };
        let source = "eventsSimple() {\n    logInfo({ message: \"hello\" })\n}\n";

        let written = call(
            "write_flowscript",
            json!({ "draft_id": "source-sdk", "source": source }),
        );
        let written: Value = serde_json::from_str(&written.text_result_for_llm)
            .expect("write returns structured source response");
        assert_eq!(written["revision"], 0);
        assert_eq!(written["source"], source);

        let checked = call(
            "check_flowscript",
            json!({ "draft_id": "source-sdk", "expected_revision": 0 }),
        );
        let checked: Value = serde_json::from_str(&checked.text_result_for_llm)
            .expect("check returns structured source response");
        assert_eq!(checked["status"], "valid", "{checked:#}");
        assert_eq!(checked["source"], source);

        let committed = call(
            "commit_flowscript",
            json!({ "draft_id": "source-sdk", "expected_revision": 0 }),
        );
        let committed: Value = serde_json::from_str(&committed.text_result_for_llm)
            .expect("commit returns structured source response");
        assert_eq!(committed["status"], "queued", "{committed:#}");
        assert_eq!(committed["source"], source);
        assert!(committed.get("commands").is_none());

        let mut queued = queue.lock().expect("command queue lock");
        let (commands, token) = queued.take_delivery();
        assert!(!commands.is_empty());
        let token = token.expect("queued source batch owns an exact review token");
        drop(queued);
        assert_eq!(
            workspace.lock().expect("workspace lock").as_deref(),
            Some(source)
        );
        let store = retained_flow_ir_draft_store(&board_key).expect("retained board store");
        assert!(store.release_commit_if_matches(
            &token.draft_id,
            token.revision,
            &token.base_fingerprint,
            &token.claim_id,
        ));

        if let Ok(mut stores) = FLOW_IR_DRAFT_STORES.lock() {
            stores.remove(&board_key);
        }
    }

    fn typed_program() -> FlowIrProgram {
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
                            value: FlowIrLiteral::String("hello".to_string()),
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

    fn committed_store(draft_id: &str) -> (Arc<FlowIrDraftStore>, Board, Vec<NodeMetadata>) {
        let store = Arc::new(FlowIrDraftStore::new());
        let board = empty_board("typed-queue-board");
        let catalog = typed_catalog();
        store.begin(
            &board,
            &catalog,
            BeginFlowIrDraftArgs {
                draft_id: draft_id.to_string(),
                replace_existing: false,
                expected_modules: vec!["eventsSimple".to_string()],
                capability_plan: FlowCapabilityPlanRequest {
                    requirements: vec![FlowCapabilityRequirement {
                        id: "format_message".to_string(),
                        intent: "format a message".to_string(),
                        required: true,
                        exact_node_type: Some("string_format".to_string()),
                        inputs: Vec::new(),
                        outputs: Vec::new(),
                    }],
                    modules: vec![FlowModuleEstimate {
                        name: "eventsSimple".to_string(),
                        kind: FlowModuleKind::Event,
                        estimated_nodes: 1,
                    }],
                },
                mode: FlowIrDraftMode::Additive,
                program: typed_program(),
            },
        );
        (store, board, catalog)
    }

    #[test]
    fn built_in_core_and_native_apply_resolve_the_same_retained_store() {
        let board_key = format!(
            "flow-ir-built-in-store-test-{:?}",
            std::thread::current().id()
        );
        let board = empty_board(&board_key);
        let built_in = retained_flow_ir_draft_store_for_board(&board)
            .expect("built-in core path acquires board-scoped draft store");
        let atomic_apply = retained_flow_ir_draft_store(&board_key)
            .expect("native Apply resolves the originating draft store");

        assert!(Arc::ptr_eq(&built_in, &atomic_apply));

        if let Ok(mut stores) = FLOW_IR_DRAFT_STORES.lock() {
            stores.remove(&board_key);
        }
    }

    fn commit_args(draft_id: &str) -> CommitFlowIrDraftArgs {
        CommitFlowIrDraftArgs {
            draft_id: draft_id.to_string(),
            expected_revision: 0,
            allow_deletions: false,
            remove_node_ids: Vec::new(),
            remove_variable_ids: Vec::new(),
            remove_layer_ids: Vec::new(),
            remove_comment_ids: Vec::new(),
            use_best_candidate: false,
        }
    }

    fn commit_token(
        board: &Board,
        draft_id: &str,
        committed: &flow_like::flow::copilot::FlowIrCommitResult,
    ) -> FlowIrCommitToken {
        FlowIrCommitToken {
            board_id: board.id.clone(),
            draft_id: draft_id.to_string(),
            revision: committed.revision.expect("queued commit revision"),
            base_fingerprint: committed
                .base_fingerprint
                .clone()
                .expect("queued commit base fingerprint"),
            claim_id: committed
                .claim_id
                .clone()
                .expect("queued commit claim generation"),
            requires_destructive_approval: false,
        }
    }

    #[test]
    fn typed_draft_store_survives_tool_surface_recreation_for_same_board() {
        let board_key = format!("flow-ir-cache-test-{:?}", std::thread::current().id());
        if let Ok(mut stores) = FLOW_IR_DRAFT_STORES.lock() {
            stores.remove(&board_key);
        }

        let board = empty_board(&board_key);
        let first =
            persisted_flow_ir_draft_store(&board_key, &board).expect("first board cache lease");
        let second =
            persisted_flow_ir_draft_store(&board_key, &board).expect("second board cache lease");
        assert!(Arc::ptr_eq(&first, &second));

        if let Ok(mut stores) = FLOW_IR_DRAFT_STORES.lock() {
            stores.remove(&board_key);
        }
    }

    #[test]
    fn typed_tool_touch_refreshes_or_replaces_the_board_store_lease() {
        let board_key = format!("flow-ir-touch-test-{:?}", std::thread::current().id());
        let stale_store = Arc::new(FlowIrDraftStore::new());
        if let Ok(mut stores) = FLOW_IR_DRAFT_STORES.lock() {
            stores.insert(
                board_key.clone(),
                CachedFlowIrDraftStore {
                    store: stale_store,
                    last_accessed: Instant::now()
                        .checked_sub(FLOW_IR_DRAFT_STORE_TTL + Duration::from_secs(1))
                        .expect("test instant supports TTL subtraction"),
                    pending_since: None,
                },
            );
        }

        let replacement = Arc::new(FlowIrDraftStore::new());
        let active = touch_persisted_flow_ir_draft_store(&board_key, replacement.clone(), None)
            .expect("expired lease can be replaced");
        assert!(Arc::ptr_eq(&active, &replacement));

        let ignored_fallback = Arc::new(FlowIrDraftStore::new());
        let touched = touch_persisted_flow_ir_draft_store(&board_key, ignored_fallback, None)
            .expect("active lease can be touched");
        assert!(Arc::ptr_eq(&touched, &replacement));

        if let Ok(mut stores) = FLOW_IR_DRAFT_STORES.lock() {
            stores.remove(&board_key);
        }
    }

    #[test]
    fn stale_tool_surface_binding_does_not_cross_store_epochs() {
        let board_key = format!("flow-ir-epoch-test-{:?}", std::thread::current().id());
        let bound_store = Arc::new(FlowIrDraftStore::new());
        let active_store = Arc::new(FlowIrDraftStore::new());
        if let Ok(mut stores) = FLOW_IR_DRAFT_STORES.lock() {
            stores.insert(
                board_key.clone(),
                CachedFlowIrDraftStore {
                    store: active_store,
                    last_accessed: Instant::now(),
                    pending_since: None,
                },
            );
        }

        assert!(matches!(
            touch_bound_flowscript_draft_store(&board_key, bound_store),
            Err(FlowIrDraftStoreAccessError::EpochChanged)
        ));

        if let Ok(mut stores) = FLOW_IR_DRAFT_STORES.lock() {
            stores.remove(&board_key);
        }
    }

    #[test]
    fn pending_commit_store_is_exempt_from_ttl_expiration() {
        let board_key = format!("flow-ir-pending-ttl-test-{:?}", std::thread::current().id());
        let draft_id = format!("pending-ttl-{:?}", std::thread::current().id());
        let (pending_store, board, catalog) = committed_store(&draft_id);
        let committed = pending_store.commit(&board, &catalog, commit_args(&draft_id));
        assert_eq!(committed.status, "queued", "{committed:#?}");
        assert!(pending_store.has_pending_commit());
        if let Ok(mut stores) = FLOW_IR_DRAFT_STORES.lock() {
            stores.insert(
                board_key.clone(),
                CachedFlowIrDraftStore {
                    store: pending_store.clone(),
                    last_accessed: Instant::now()
                        .checked_sub(FLOW_IR_DRAFT_STORE_TTL + Duration::from_secs(1))
                        .expect("test instant supports TTL subtraction"),
                    pending_since: None,
                },
            );
        }

        let active = touch_persisted_flow_ir_draft_store(
            &board_key,
            Arc::new(FlowIrDraftStore::new()),
            None,
        )
        .expect("pending lease remains retained");
        assert!(Arc::ptr_eq(&active, &pending_store));

        let mut advanced_board = board.clone();
        let applied_variable = Variable::new(
            "appliedTypedCommandBatch",
            VariableType::String,
            ValueType::Normal,
        );
        advanced_board
            .variables
            .insert(applied_variable.id.clone(), applied_variable);
        if let Ok(mut stores) = FLOW_IR_DRAFT_STORES.lock()
            && let Some(cached) = stores.get_mut(&board_key)
        {
            cached.last_accessed = Instant::now()
                .checked_sub(FLOW_IR_DRAFT_STORE_TTL + Duration::from_secs(1))
                .expect("test instant supports TTL subtraction");
        }
        let replacement = Arc::new(FlowIrDraftStore::new());
        let after_observation = touch_persisted_flow_ir_draft_store(
            &board_key,
            replacement.clone(),
            Some(&advanced_board),
        )
        .expect("observation keeps an unresolved review pinned");
        assert!(pending_store.has_pending_commit());
        assert!(Arc::ptr_eq(&after_observation, &pending_store));

        let token = commit_token(&board, &draft_id, &committed);
        assert!(pending_store.acknowledge_applied_commit(
            &advanced_board,
            &token.draft_id,
            token.revision,
            &token.base_fingerprint,
            &token.claim_id,
        ));
        assert!(!pending_store.has_pending_commit());
        if let Ok(mut stores) = FLOW_IR_DRAFT_STORES.lock()
            && let Some(cached) = stores.get_mut(&board_key)
        {
            cached.last_accessed = Instant::now()
                .checked_sub(FLOW_IR_DRAFT_STORE_TTL + Duration::from_secs(1))
                .expect("test instant supports TTL subtraction");
        }
        let after_apply = touch_persisted_flow_ir_draft_store(
            &board_key,
            replacement.clone(),
            Some(&advanced_board),
        )
        .expect("resolved expired lease can be replaced");
        assert!(Arc::ptr_eq(&after_apply, &replacement));
        // Permanent revision idempotency remains even though the explicit applied disposition
        // cleared the transient board reservation and cache pin.
        assert_eq!(
            pending_store
                .commit(&board, &catalog, commit_args(&draft_id))
                .status,
            "already_queued"
        );

        if let Ok(mut stores) = FLOW_IR_DRAFT_STORES.lock() {
            stores.remove(&board_key);
        }
    }

    #[test]
    fn pending_commit_store_expires_after_absolute_review_lease() {
        let board_key = format!(
            "flow-ir-pending-review-lease-test-{:?}",
            std::thread::current().id()
        );
        let draft_id = format!("pending-review-lease-{:?}", std::thread::current().id());
        let (pending_store, board, catalog) = committed_store(&draft_id);
        assert_eq!(
            pending_store
                .commit(&board, &catalog, commit_args(&draft_id))
                .status,
            "queued"
        );
        let now = Instant::now();
        if let Ok(mut stores) = FLOW_IR_DRAFT_STORES.lock() {
            stores.insert(
                board_key.clone(),
                CachedFlowIrDraftStore {
                    store: pending_store.clone(),
                    // Recent traffic must not extend the absolute unresolved-review lease.
                    last_accessed: now,
                    pending_since: Some(
                        now.checked_sub(FLOW_IR_PENDING_REVIEW_TTL + Duration::from_secs(1))
                            .expect("test instant supports review lease subtraction"),
                    ),
                },
            );
        }

        let replacement = Arc::new(FlowIrDraftStore::new());
        let active =
            touch_persisted_flow_ir_draft_store(&board_key, replacement.clone(), Some(&board))
                .expect("an abandoned pending review releases its bounded cache slot");
        assert!(Arc::ptr_eq(&active, &replacement));
        assert!(!Arc::ptr_eq(&active, &pending_store));

        if let Ok(mut stores) = FLOW_IR_DRAFT_STORES.lock() {
            stores.remove(&board_key);
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn current_board_operation_prefers_the_live_registry_board() {
        let captured = empty_board("live-board-test");
        let mut updated = captured.clone();
        updated.name = "Live".to_string();
        let live = Arc::new(AsyncMutex::new(updated));

        let observed = with_current_board(&captured, Some(&live), |board| board.name.clone());
        assert_eq!(observed, "Live");
        let fallback = with_current_board(&captured, None, |board| board.name.clone());
        assert_eq!(fallback, "Captured");
    }

    #[test]
    fn hard_cache_cap_rejects_a_new_board_when_every_slot_is_pending() {
        let draft_id = "capacity-pending";
        let (pending_store, board, catalog) = committed_store(draft_id);
        assert_eq!(
            pending_store
                .commit(&board, &catalog, commit_args(draft_id))
                .status,
            "queued"
        );
        let now = Instant::now();
        let mut stores = (0..MAX_PERSISTED_FLOW_IR_DRAFT_STORES)
            .map(|index| {
                (
                    format!("pending-board-{index}"),
                    CachedFlowIrDraftStore {
                        store: pending_store.clone(),
                        last_accessed: now,
                        pending_since: None,
                    },
                )
            })
            .collect::<HashMap<_, _>>();

        assert!(!reclaim_flow_ir_draft_store_slot(&mut stores));
        assert_eq!(stores.len(), MAX_PERSISTED_FLOW_IR_DRAFT_STORES);
    }

    #[test]
    fn stale_delivery_generation_cannot_release_an_exact_revision_retry() {
        let draft_id = "aba-safe-delivery";
        let (store, board, catalog) = committed_store(draft_id);
        let args = commit_args(draft_id);
        let first = store.commit(&board, &catalog, args.clone());
        assert_eq!(first.status, "queued", "{first:#?}");
        let first_token = commit_token(&board, draft_id, &first);
        assert!(release_retained_commit_claim(&store, &first_token));

        let retry = store.commit(&board, &catalog, args);
        assert_eq!(retry.status, "queued", "{retry:#?}");
        let retry_token = commit_token(&board, draft_id, &retry);
        assert_ne!(first_token.claim_id, retry_token.claim_id);
        assert!(!release_retained_commit_claim(&store, &first_token));
        assert!(store.pending_commit_matches(
            &retry_token.draft_id,
            retry_token.revision,
            &retry_token.base_fingerprint,
            &retry_token.claim_id,
        ));
    }

    #[test]
    fn typed_ir_parse_hint_uses_the_canonical_tagged_shapes() {
        let hint = typed_ir_schema_hint();
        assert_eq!(hint["type_object"]["data_type"], "string");
        assert_eq!(hint["literal_boolean"]["value"]["type"], "boolean");
        assert_eq!(hint["literal_integer"]["value"]["type"], "integer");
        assert_eq!(hint["value_ref"]["kind"], "ref");
        assert_eq!(hint["call_function_step"]["kind"], "call_function");
        assert!(hint["if_step"].get("then_steps").is_some());
    }

    #[test]
    fn dropping_an_undrained_typed_batch_releases_its_commit_claim() {
        let draft_id = "abandoned-host-queue";
        let (store, board, catalog) = committed_store(draft_id);
        let args = commit_args(draft_id);
        let committed = store.commit(&board, &catalog, args.clone());
        assert_eq!(committed.status, "queued", "{committed:#?}");
        let flowscript = committed
            .flowscript
            .clone()
            .expect("typed commit carries compiled FlowScript");
        let workspace = Arc::new(Mutex::new(Some(flowscript.clone())));
        let token = commit_token(&board, draft_id, &committed);

        let mut queue = SideEffectCommandQueue::default();
        assert!(queue.extend_retained_commit(
            committed.commands,
            store.clone(),
            token,
            Some(workspace.clone()),
            Some(flowscript),
        ));
        drop(queue);

        assert_eq!(*workspace.lock().expect("workspace lock"), None);
        assert_eq!(store.commit(&board, &catalog, args).status, "queued");
    }

    #[test]
    fn retained_batch_is_not_streamable_before_atomic_final_delivery() {
        let draft_id = "delivered-host-queue";
        let (store, board, catalog) = committed_store(draft_id);
        let args = commit_args(draft_id);
        let committed = store.commit(&board, &catalog, args.clone());
        assert_eq!(committed.status, "queued", "{committed:#?}");
        let flowscript = committed
            .flowscript
            .clone()
            .expect("typed commit carries compiled FlowScript");
        let workspace = Arc::new(Mutex::new(Some(flowscript.clone())));
        let token = commit_token(&board, draft_id, &committed);

        let expected_count = committed.commands.len();
        let mut queue = SideEffectCommandQueue::default();
        assert!(queue.extend_retained_commit(
            committed.commands,
            store.clone(),
            token,
            Some(workspace.clone()),
            Some(flowscript.clone()),
        ));
        assert!(queue.drain_streamable().is_empty());
        assert_eq!(queue.commands.len(), expected_count);
        drop(queue);

        assert_eq!(*workspace.lock().expect("workspace lock"), None);
        assert_eq!(store.commit(&board, &catalog, args).status, "queued");
    }

    #[test]
    fn retained_claim_never_absorbs_an_unrelated_queued_command() {
        let draft_id = "exact-batch-queue-conflict";
        let (store, board, catalog) = committed_store(draft_id);
        let args = commit_args(draft_id);
        let committed = store.commit(&board, &catalog, args.clone());
        assert_eq!(committed.status, "queued", "{committed:#?}");
        let token = commit_token(&board, draft_id, &committed);

        let unrelated = BoardCommand::RemoveNode {
            node_id: "unrelated-direct-command".to_string(),
            summary: None,
        };
        let mut queue = SideEffectCommandQueue::default();
        assert!(queue.extend([unrelated]));
        assert!(!queue.extend_retained_commit(
            committed.commands,
            store.clone(),
            token,
            None,
            committed.flowscript,
        ));
        assert!(queue.commit_claim.is_none());
        assert_eq!(queue.drain_streamable().len(), 1);
        assert_eq!(
            store.commit(&board, &catalog, args).status,
            "queued",
            "rejecting a mixed host batch must reopen only the retained claim"
        );
    }

    #[test]
    fn direct_command_is_refused_after_a_retained_claim_is_queued() {
        let draft_id = "inverse-exact-batch-queue-conflict";
        let (store, board, catalog) = committed_store(draft_id);
        let args = commit_args(draft_id);
        let committed = store.commit(&board, &catalog, args.clone());
        assert_eq!(committed.status, "queued", "{committed:#?}");
        let expected_count = committed.commands.len();
        let expected_token = commit_token(&board, draft_id, &committed);

        let mut queue = SideEffectCommandQueue::default();
        assert!(queue.extend_retained_commit(
            committed.commands,
            store.clone(),
            expected_token.clone(),
            None,
            committed.flowscript,
        ));
        assert!(!queue.extend([BoardCommand::RemoveNode {
            node_id: "late-unrelated-direct-command".to_string(),
            summary: None,
        }]));

        let (commands, token) = queue.take_delivery();
        assert_eq!(commands.len(), expected_count);
        assert_eq!(token, Some(expected_token.clone()));
        assert!(store.pending_commit_matches(
            &expected_token.draft_id,
            expected_token.revision,
            &expected_token.base_fingerprint,
            &expected_token.claim_id,
        ));
    }

    #[test]
    fn cancellation_before_atomic_delivery_releases_claim_and_commands_together() {
        let draft_id = "cancelled-after-stream-drain";
        let (store, board, catalog) = committed_store(draft_id);
        let args = commit_args(draft_id);
        let committed = store.commit(&board, &catalog, args.clone());
        assert_eq!(committed.status, "queued", "{committed:#?}");
        let token = commit_token(&board, draft_id, &committed);

        let mut queue = SideEffectCommandQueue::default();
        assert!(queue.extend_retained_commit(
            committed.commands,
            store.clone(),
            token,
            None,
            committed.flowscript,
        ));
        assert!(queue.drain_streamable().is_empty());

        // No renderer can observe claimed commands before owning their token. Cancellation abandons
        // both sides of the atomic delivery and reopens the exact retained revision.
        queue.abandon();
        let (commands, token) = queue.take_delivery();
        assert!(commands.is_empty());
        assert!(token.is_none());
        assert_eq!(store.commit(&board, &catalog, args).status, "queued");
    }

    #[test]
    fn final_response_transfer_keeps_the_exact_commit_pending() {
        let draft_id = "delivered-final-response";
        let (store, board, catalog) = committed_store(draft_id);
        let args = commit_args(draft_id);
        let committed = store.commit(&board, &catalog, args.clone());
        assert_eq!(committed.status, "queued", "{committed:#?}");
        let flowscript = committed
            .flowscript
            .clone()
            .expect("typed commit carries compiled FlowScript");
        let workspace = Arc::new(Mutex::new(Some(flowscript.clone())));
        let expected_token = commit_token(&board, draft_id, &committed);

        let mut queue = SideEffectCommandQueue::default();
        assert!(queue.extend_retained_commit(
            committed.commands,
            store.clone(),
            expected_token.clone(),
            Some(workspace.clone()),
            Some(flowscript.clone()),
        ));
        let (commands, token) = queue.take_delivery();
        assert!(!commands.is_empty());
        assert_eq!(token, Some(expected_token));
        drop(queue);

        assert_eq!(
            workspace.lock().expect("workspace lock").as_deref(),
            Some(flowscript.as_str())
        );
        assert_eq!(
            store.commit(&board, &catalog, args).status,
            "already_queued"
        );
    }

    #[test]
    fn malformed_claim_without_commands_fails_closed_and_reopens_revision() {
        let draft_id = "malformed-atomic-delivery";
        let (store, board, catalog) = committed_store(draft_id);
        let args = commit_args(draft_id);
        let committed = store.commit(&board, &catalog, args.clone());
        assert_eq!(committed.status, "queued", "{committed:#?}");
        let token = commit_token(&board, draft_id, &committed);

        let mut queue = SideEffectCommandQueue::default();
        assert!(queue.extend_retained_commit(
            committed.commands,
            store.clone(),
            token,
            None,
            committed.flowscript,
        ));
        queue.commands.clear();

        let (commands, token) = queue.take_delivery();
        assert!(commands.is_empty());
        assert!(token.is_none());
        assert_eq!(store.commit(&board, &catalog, args).status, "queued");
    }

    #[test]
    fn draft_snapshot_persist_and_hydrate_roundtrip() {
        let board = empty_board("snapshot-board");
        let catalog = typed_catalog();
        let store = FlowIrDraftStore::new();
        let written = store.write_flowscript(
            &board,
            &catalog,
            WriteFlowScriptArgs {
                draft_id: "durable".to_string(),
                replace_existing: false,
                mode: FlowIrDraftMode::Additive,
                source: "eventsSimple() {\n    logInfo({ message: \"hello\" })\n}\n".to_string(),
                allow_scope_reduction: false,
            },
        );
        assert_eq!(written.status, "draft_started", "{written:#?}");

        let dir = std::env::temp_dir().join(format!(
            "flow-like-draft-snapshot-roundtrip-{}",
            std::process::id()
        ));
        let path = dir.join("snapshot-board.drafts.json");
        persist_flow_ir_draft_snapshot(&path, &store);
        assert!(path.exists());

        let restored = FlowIrDraftStore::new();
        hydrate_flow_ir_draft_store_from_path(&path, &board, &restored);
        assert!(restored.has_editable_draft_for_board(&board.id));

        // A board that moved past the snapshot's base fingerprint must not revive stale drafts.
        let mut advanced = board.clone();
        let mut variable = Variable::new("marker", VariableType::String, ValueType::Normal);
        variable.id = "marker".to_string();
        advanced.variables.insert(variable.id.clone(), variable);
        let stale = FlowIrDraftStore::new();
        hydrate_flow_ir_draft_store_from_path(&path, &advanced, &stale);
        assert!(!stale.has_editable_draft_for_board(&board.id));

        // Persisting an empty store removes the snapshot instead of leaving a stale file.
        persist_flow_ir_draft_snapshot(&path, &FlowIrDraftStore::new());
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Component types with a detailed schema page in
    /// `flow_like::a2ui::copilot::get_component_schema` (advertised by the
    /// get_component_schema tool description).
    const SCHEMA_PAGE_TYPES: &[&str] = &[
        "column",
        "row",
        "grid",
        "text",
        "button",
        "feedback",
        "appLink",
        "card",
        "userProfile",
        "textField",
        "select",
        "image",
        "icon",
        "diffView",
        "calendar",
        "gantt",
        "checkbox",
        "switch",
        "tabs",
        "modal",
    ];

    /// Every component type advertised in the a2ui docs (the quick-reference
    /// catalog embedded in the system prompt plus the detailed schema pages).
    fn documented_component_types() -> Vec<String> {
        let mut types = Vec::new();
        for line in flow_like::a2ui::copilot::COMPONENT_CATALOG.lines() {
            if let Some(rest) = line.trim().strip_prefix("- `")
                && let Some(end) = rest.find('`')
            {
                types.push(rest[..end].to_string());
            }
        }
        assert!(
            types.len() > 30,
            "catalog parse looks broken: {} types",
            types.len()
        );
        for schema_type in SCHEMA_PAGE_TYPES {
            if !types.iter().any(|t| t == schema_type) {
                types.push(schema_type.to_string());
            }
        }
        types
    }

    fn representative_prop_value(prop: &str) -> Value {
        match prop {
            "width" | "height" | "x" | "y" | "ratio" | "maxValue" => json!({"literalNumber": 100}),
            "checked" | "open" => json!({"literalBool": false}),
            "options" => json!({"literalOptions": [{"value": "a", "label": "A"}]}),
            "events" | "tasks" => json!({"literalJson": "[]"}),
            "shapeType" => json!({"literalString": "rectangle"}),
            _ => json!({"literalString": "x"}),
        }
    }

    #[test]
    fn emit_ui_accepts_every_documented_component_type() {
        for comp_type in documented_component_types() {
            let props = known_props_for_type(&comp_type);
            assert!(
                props.is_some(),
                "documented component type '{comp_type}' is rejected by known_props_for_type — validator drifted from the docs"
            );

            let mut component = serde_json::Map::new();
            component.insert("type".to_string(), json!(comp_type));
            component.insert("hidden".to_string(), json!({"literalBool": false}));
            for required in required_props_for_type(&comp_type) {
                component.insert(required.to_string(), representative_prop_value(required));
            }

            let components = json!([{ "id": "root", "component": Value::Object(component) }]);
            let (_, errors) = validate_ui_components("root", &json!({}), &components);
            assert!(
                errors.is_empty(),
                "documented component type '{comp_type}' fails emit_ui validation: {errors:?}"
            );
        }
    }

    #[test]
    fn emit_ui_accepts_all_known_props_for_documented_types() {
        for comp_type in documented_component_types() {
            let Some(props) = known_props_for_type(&comp_type) else {
                continue;
            };
            let mut component = serde_json::Map::new();
            component.insert("type".to_string(), json!(comp_type));
            for prop in props {
                component.insert(prop.to_string(), representative_prop_value(prop));
            }
            // Plain-typed reference props must point at a real component.
            component.remove("baseComponentId");
            component.remove("contentComponentId");
            for required in required_props_for_type(&comp_type) {
                component
                    .entry(required.to_string())
                    .or_insert_with(|| representative_prop_value(required));
            }

            let components = json!([{ "id": "root", "component": Value::Object(component) }]);
            let (_, errors) = validate_ui_components("root", &json!({}), &components);
            let unknown_prop_errors: Vec<&String> = errors
                .iter()
                .filter(|error| error.contains("unknown prop"))
                .collect();
            assert!(
                unknown_prop_errors.is_empty(),
                "'{comp_type}' rejects props it declares as known: {unknown_prop_errors:?}"
            );
        }
    }
}
