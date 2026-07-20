//! SCRATCH — drives the FlowPilot-generated edge-case document through the real
//! write → check → commit → apply → re-reconcile loop.
//!
//! Copy to packages/catalog/tests/zz_edgecase_probe.rs to run:
//!   cargo test -p flow-like-catalog --test zz_edgecase_probe -- --nocapture
//! Delete again afterwards — this is a diagnostic harness, not a committed test.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::SystemTime;

use flow_like::flow::ast::{
    RenderOptions, apply_board_commands_to_board, board_to_flowscript, reconcile_text_with_catalog,
};
use flow_like::flow::board::{Board, ExecutionMode, ExecutionStage};
use flow_like::flow::copilot::{
    BoardCommand, CheckFlowScriptArgs, CommitFlowScriptArgs, FlowIrDraftMode, FlowIrDraftStore,
    NodeMetadata, WriteFlowScriptArgs, node_to_metadata,
};
use flow_like::flow::execution::LogLevel;
use flow_like::flow::node::{Node, NodeLogic};
use flow_like::state::{FlowLikeConfig, FlowLikeState};
use flow_like::utils::http::HTTPClient;
use flow_like_catalog::CatalogBuilder;
use flow_like_storage::object_store::path::Path;

struct CatalogFixture {
    logic: Vec<Arc<dyn NodeLogic>>,
    nodes: Vec<Node>,
    metadata: Vec<NodeMetadata>,
}

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
    Board {
        id: id.to_string(),
        name: format!("Probe Board {id}"),
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
        created_at: SystemTime::UNIX_EPOCH,
        updated_at: SystemTime::UNIX_EPOCH,
        parent: None,
        board_dir: Path::default(),
        logic_nodes: HashMap::new(),
        app_state: None,
    }
}

async fn catalog_state() -> Arc<FlowLikeState> {
    let state = Arc::new(FlowLikeState::new(
        FlowLikeConfig::new(),
        HTTPClient::new_without_refetch(),
    ));
    let registry = state.node_registry();
    registry.write().await.push_nodes(FIXTURE.logic.clone());
    state
}

fn source() -> String {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tmp/flowpilot.edgecase.flowscript"
    );
    std::fs::read_to_string(path).expect("fixture readable")
}

fn dump<T: std::fmt::Debug>(label: &str, items: &[T], limit: usize) {
    println!("--- {} ({}) ---", label, items.len());
    for d in items.iter().take(limit) {
        let s = format!("{d:?}");
        println!("  * {}", &s[..s.len().min(600)]);
    }
    if items.len() > limit {
        println!("  … {} more", items.len() - limit);
    }
}

/// Successively repair the known blockers and report which one fires at each stage.
fn staged_sources(base: &str) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    out.push(("0-original", base.to_string()));

    // Stage 1: drop the `tools: [...]` fn-ref args (A2).
    let mut s1 = String::new();
    for line in base.lines() {
        if let Some(idx) = line.find(", tools: [") {
            let close = line[idx..].find(']').map(|i| idx + i + 1).unwrap_or(line.len());
            let mut fixed = String::from(&line[..idx]);
            fixed.push_str(&line[close..]);
            s1.push_str(&fixed);
        } else {
            s1.push_str(line);
        }
        s1.push('\n');
    }
    out.push(("1-no-fn-refs", s1.clone()));

    // Stage 2: also stop returning a bare function parameter (A1).
    let s2 = s1.replace(
        "    return resolvedRole.role, userId, canRead, canSearch, canWrite, canMove, canAnalytics",
        "    const actorCopy = valToString({ value: userId })\n    return resolvedRole.role, actorCopy, canRead, canSearch, canWrite, canMove, canAnalytics",
    );
    out.push(("2-no-param-return", s2.clone()));

    // Stage 3: also avoid the chart mode that deletes the `data` pin (A3).
    let s3 = s2.replace("format: \"CSV\", data:", "format: \"JSON\", data:");
    out.push(("3-chart-json-mode", s3));
    out
}

#[tokio::test]
async fn probe_full_loop() {
    let fixture = &*FIXTURE;
    let base = source();
    for (stage, src) in staged_sources(&base) {
        println!("\n######## STAGE {stage} ########");
        run_stage(fixture, &stage.to_string(), &src).await;
    }
}

async fn run_stage(fixture: &CatalogFixture, stage: &str, src_in: &str) {
    let src = src_in.to_string();
    let draft = format!("probe-draft-{stage}");
    let store = FlowIrDraftStore::new();
    let board = empty_board("probe");

    let write = store.write_flowscript(
        &board,
        &fixture.metadata,
        WriteFlowScriptArgs {
            draft_id: draft.clone(),
            replace_existing: false,
            mode: FlowIrDraftMode::Additive,
            source: src.clone(),
            allow_scope_reduction: false,
        },
    );
    println!(
        "WRITE status={:?} code={:?} revision={:?}",
        write.status, write.code, write.revision
    );
    dump("write diagnostics", &write.diagnostics, 40);
    let revision = write.revision.unwrap_or(0);

    let check = store.check_flowscript(
        &board,
        &fixture.metadata,
        CheckFlowScriptArgs {
            draft_id: draft.clone(),
            expected_revision: revision,
        },
    );
    println!("CHECK status={:?} code={:?}", check.status, check.code);
    dump("check diagnostics", &check.diagnostics, 60);

    let commit = store.commit_flowscript(
        &board,
        &fixture.metadata,
        CommitFlowScriptArgs {
            draft_id: draft.clone(),
            expected_revision: check.revision.unwrap_or(revision),
            allow_deletions: false,
            remove_node_ids: Vec::new(),
            remove_variable_ids: Vec::new(),
            remove_layer_ids: Vec::new(),
            remove_comment_ids: Vec::new(),
        },
    );
    println!("COMMIT status={:?} code={:?}", commit.status, commit.code);
    dump("commit diagnostics", &commit.diagnostics, 40);
    let commands: Vec<BoardCommand> = commit.commands.clone();
    println!("COMMIT queued {} commands", commands.len());
    if commands.is_empty() {
        println!("!! no commands queued — stopping");
        return;
    }

    let state = catalog_state().await;
    let mut applied_board = board.clone();
    let applied =
        apply_board_commands_to_board(&mut applied_board, commands, &fixture.nodes, state, None)
            .await;
    let applied = match applied {
        Ok(a) => a,
        Err(e) => {
            println!("!! APPLY FAILED: {e}");
            return;
        }
    };
    dump("apply diagnostics", &applied.diagnostics, 40);
    println!(
        "APPLIED board: {} nodes, {} layers, {} variables",
        applied_board.nodes.len(),
        applied_board.layers.len(),
        applied_board.variables.len()
    );

    let anchored = board_to_flowscript(
        &applied_board,
        &RenderOptions {
            anchors: true,
            ..RenderOptions::default()
        },
    );
    std::fs::write(
        format!("/tmp/claude-1000/applied.{stage}.anchored.flowscript"),
        &anchored,
    )
    .ok();
    println!(
        "RENDERED BACK: {} lines (original {} lines)",
        anchored.lines().count(),
        src.lines().count()
    );

    let again = reconcile_text_with_catalog(&applied_board, &anchored, &fixture.metadata);
    println!(
        "SELF-RECONCILE: {} commands, {} diagnostics",
        again.commands.len(),
        again.diagnostics.len()
    );
    dump("self-reconcile diagnostics", &again.diagnostics, 40);
    for c in again.commands.iter().take(8) {
        let s = format!("{c:?}");
        println!("  cmd: {}", &s[..s.len().min(400)]);
    }

    let plain = board_to_flowscript(&applied_board, &RenderOptions::default());
    std::fs::write(
        format!("/tmp/claude-1000/applied.{stage}.plain.flowscript"),
        &plain,
    )
    .ok();
}
