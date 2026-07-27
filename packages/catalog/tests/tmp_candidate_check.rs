use flow_like::flow::ast::reconcile_text_with_catalog;
use flow_like::flow::board::Board;
use flow_like::flow::copilot::{NodeMetadata, node_to_metadata};
use flow_like::flow::node::{Node, NodeLogic};
use flow_like_catalog::CatalogBuilder;
use flow_like_storage::object_store::path::Path;
use std::sync::Arc;

#[test]
#[ignore = "manual"]
fn check_candidate() {
    let source = std::fs::read_to_string(
        "/private/tmp/claude-501/-Users-felix-Git-flow-like/ac916e02-ebe4-47d9-bf07-194ae27f149a/scratchpad/mail2-last-candidate.flow",
    ).expect("candidate");
    let logic: Vec<Arc<dyn NodeLogic>> = CatalogBuilder::new().build();
    let nodes: Vec<Node> = logic.iter().map(|l| l.get_node()).collect();
    let metadata: Vec<NodeMetadata> = nodes.iter().map(node_to_metadata).collect();
    let board = Board::new_detached(Some("cand".into()), Path::default());
    let result = reconcile_text_with_catalog(&board, &source, &metadata);
    for d in &result.diagnostics {
        println!("DIAG: {d}");
    }
    println!("commands: {}", result.commands.len());
}
