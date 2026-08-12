//! Compiles every fenced FlowScript example that ships in the copilot prompts against the real
//! catalog + reconciler. A prompt example that does not compile teaches the model a broken
//! pattern, so drift here must fail CI.

use std::sync::{Arc, LazyLock};

use flow_like::flow::ast::reconcile_text_with_catalog;
use flow_like::flow::board::Board;
use flow_like::flow::copilot::{NodeMetadata, node_to_metadata};
use flow_like::flow::node::{Node, NodeLogic};
use flow_like_catalog::CatalogBuilder;
use flow_like_storage::object_store::path::Path;

struct CatalogFixture {
    metadata: Vec<NodeMetadata>,
}

static FIXTURE: LazyLock<CatalogFixture> = LazyLock::new(|| {
    let logic: Vec<Arc<dyn NodeLogic>> = CatalogBuilder::new().build();
    let nodes: Vec<Node> = logic.iter().map(|logic| logic.get_node()).collect();
    let metadata: Vec<NodeMetadata> = nodes.iter().map(node_to_metadata).collect();
    CatalogFixture { metadata }
});

fn assert_example_compiles(label: &str, source: &str) {
    let mut board = Board::new_detached(Some(format!("example-{label}")), Path::default());
    board.name = format!("Example {label}");
    let result = reconcile_text_with_catalog(&board, source, &FIXTURE.metadata);
    assert!(
        result.diagnostics.is_empty(),
        "prompt example `{label}` does not compile:\n{:#?}\nsource:\n{source}",
        result.diagnostics
    );
    assert!(
        !result.commands.is_empty(),
        "prompt example `{label}` derives no board commands:\nsource:\n{source}"
    );
}

/// Extract every ```ts fenced block from a prompt const.
fn fenced_sources(prompt: &str) -> Vec<String> {
    let mut sources = Vec::new();
    let mut rest = prompt;
    while let Some(start) = rest.find("```ts\n") {
        let body = &rest[start + 6..];
        let Some(end) = body.find("```") else { break };
        sources.push(body[..end].to_string());
        rest = &body[end + 3..];
    }
    sources
}

#[test]
fn domain_examples_compile_against_the_real_catalog() {
    let sources = fenced_sources(flow_like::copilot::prompts::FLOWSCRIPT_DOMAIN_EXAMPLES);
    assert!(
        sources.len() >= 4,
        "expected at least 4 fenced domain examples, found {}",
        sources.len()
    );
    for (index, source) in sources.iter().enumerate() {
        assert_example_compiles(&format!("domain-{index}"), source);
    }
}
